# Large-File Filtered Content Streaming Follow-up

Date: 2026-07-02

Starting commit: `7c8838e` (`Reach large-file streaming target`)

Parent reports:

- Binding Surface baseline: `docs/large_file_performance_baseline.md`
- Binding Parity report: `docs/large_file_performance_binding_parity.md`
- Release Packaging report: `docs/large_file_performance_release_packaging.md`

## Verdict

Release Packaging proved the 4.0 GB / 1000s-pages target for unfiltered synthetic content streams at about 20 MB peak working set. This follow-up closes the disclosed gap for filtered page content streams. A synthetic 4.0 GiB decoded Flate-content PDF now opens, extracts text, scans images, and renders pages 1-3 under the fixed 2 GB Job Object cap with about 9-11 MiB peak working set. Real-file text outputs for the 80 MB and 703 MB samples remain SHA-256 identical to Release Packaging.

The honest remaining caveat is scope: this proves streaming decode/tokenization for page content filters. Image codecs (`DCTDecode`, `JPXDecode`, `CCITTFaxDecode`, `JBIG2Decode`) remain owned by the image pipeline and its existing caps, and a real 3-4 GB customer PDF is still needed when available.

## What Changed

1. Filtered content streams can now decode through a `Read` chain instead of a full decoded `Vec<u8>`.
   - `crates/engine/src/filters.rs:85` adds `decode_stream_lossless_reader`.
   - `crates/engine/src/filters.rs:230` composes the streaming filter chain.
   - `crates/engine/src/filters.rs:358` streams Flate/zlib or raw deflate with the existing `flate2` crate.
   - `crates/engine/src/filters.rs:808` streams PNG/TIFF predictors row-by-row.
   - LZW, RunLength, ASCIIHex, and ASCII85 are implemented as incremental readers in the same file.

2. The content read path now exposes raw content stream ranges even when `/Filter` is present.
   - `crates/engine/src/reader.rs:559` exposes bounded content stream ranges.
   - `crates/engine/src/document.rs:214` returns a page's content stream ranges.
   - `crates/engine/src/engine.rs:469` routes page content through the streaming decoder and `StreamingContentTokenizer`.
   - `crates/engine/src/content/parser.rs:32` propagates decoder I/O failures while preserving normal content-token warning behavior.

3. Synthetic fixtures can now generate Flate-compressed content without holding the decoded payload in Python memory.
   - `scripts/generate_large_pdf_ladder.py:70` adds `compress_content`.
   - `scripts/generate_large_pdf_ladder.py:103` writes `/Filter /FlateDecode` content streams from a temporary compressed payload.

## Measurement Method

The Binding Surface harness was reused unchanged:

- Worker: `target/release/examples/large_file_probe.exe`
- Wrapper: `scripts/large_file_profile.py`
- Cap: Windows Job Object, 2048 MB hard process/job memory limit.
- Memory metric: sampled peak working set.
- Runs: one large child process at a time.

Generated compressed-content fixtures:

| Fixture | Kind | On disk | Decoded content | Pages | Per-stream decoded size |
|---|---:|---:|---:|---:|---:|
| `synthetic-flate-size-1024mb-4p.pdf` | synthetic Flate content | 1.3 MB | 1.0 GiB | 4 | 256 MiB |
| `synthetic-flate-size-2048mb-8p.pdf` | synthetic Flate content | 2.6 MB | 2.0 GiB | 8 | 256 MiB |
| `synthetic-flate-size-4096mb-16p.pdf` | synthetic Flate content | 5.1 MB | 4.0 GiB | 16 | 256 MiB |

## Before vs After

All rows ran under the fixed 2048 MB cap. The before rows are the current Release Packaging code with only the generator addition, before the streaming decoder change.

| Run | Result | Operation | Decoded content | Done | Wall | Peak WS | TTFP |
|---|---:|---|---:|---:|---:|---:|---:|
| `before-flate-s1024-text` | ok | text page mode | 1.0 GiB | 4p | 1.8s | 549.6 MiB | 402 ms |
| `after-flate-s1024-text` | ok | text page mode | 1.0 GiB | 4p | 3.8s | 6.0 MiB | 941 ms |
| `before-flate-s2048-text` | ok | text page mode | 2.0 GiB | 8p | 3.0s | 549.6 MiB | 396 ms |
| `after-flate-s2048-text` | ok | text page mode | 2.0 GiB | 8p | 7.5s | 6.6 MiB | 965 ms |

The pre-fix peak tracked the largest fully decoded filtered stream (256 MiB plus parser/allocator overhead). The post-fix peak stays flat because the tokenizer consumes decoded bytes as the filter readers produce them. Wall time is slower on this deliberately compressible stress fixture; memory, not speed, is the hard constraint for this follow-up.

## Compressed Target Ladder

| Run | Result | Operation | Decoded content / pages | Done | Wall | Peak WS | TTFP |
|---|---:|---|---:|---:|---:|---:|---:|
| `after-flate-s4096-open` | ok | open | 4.0 GiB / 16p | 0 | 0.3s | 9.3 MiB | n/a |
| `after-flate-s4096-text` | ok | text page mode | 4.0 GiB / 16p | 16p | 15.0s | 9.3 MiB | 932 ms |
| `after-flate-s4096-images` | ok | image scan/extract | 4.0 GiB / 16p | 16p | 15.0s | 9.3 MiB | 922 ms |
| `after-flate-s4096-render-p1-3` | ok | render pages 1-3 | 4.0 GiB / 16p | 3p | 3.0s | 9.3 MiB | 949 ms |

Serial/parallel aggregate text on the same 4.0 GiB decoded compressed fixture:

| Run | Window | Result | Wall | Peak WS | SHA-256 |
|---|---:|---:|---:|---:|---|
| `followup-flate-s4096-serial-aggregate` | `WELLFRIENDPDF_TEXT_PARALLEL_PAGES=1` | ok | 14.9s | 9.3 MiB | `0232F63C17EA191E11ED168CBE1CFAEC43AC4B76A2986FF61D75170B62FBBD2A` |
| `followup-flate-s4096-parallel-aggregate` | default bounded window | ok | 4.8s | 10.8 MiB | `0232F63C17EA191E11ED168CBE1CFAEC43AC4B76A2986FF61D75170B62FBBD2A` |

## Real Files

| Run | Result | Operation | File | Done | Wall | Peak WS | TTFP | Release Packaging comparison |
|---|---:|---|---:|---:|---:|---:|---:|---|
| `followup-real-80mb-text` | ok | text page mode | 80.2 MB / 274p | 274p | 5.5s | 23.4 MiB | 24 ms | Release Packaging: 5.5s, 23.7 MiB |
| `followup-real-703mb-text` | ok | text page mode | 703.4 MB / 52p | 52p | 0.3s | 84.4 MiB | 90 ms | Release Packaging: 0.5s, 84.4 MiB |

Text output SHA-256 against Release Packaging:

| File | Release Packaging SHA-256 | Follow-up SHA-256 | Result |
|---|---|---|---|
| `real-80mb-pp80` | `104A47F5535AA9222F897C5FCFE920511F22BEEC21C523FC8FD9BB11FCB6CEF5` | `104A47F5535AA9222F897C5FCFE920511F22BEEC21C523FC8FD9BB11FCB6CEF5` | identical |
| `real-703mb-atlas` | `AC2C2725ECAE4D38CE077D5367B6D6E80A68DA4E51DED7869A4135F3D7293958` | `AC2C2725ECAE4D38CE077D5367B6D6E80A68DA4E51DED7869A4135F3D7293958` | identical |

## Correctness and Robustness

Differential byte tests:

- Buffered vs streaming decode is byte-identical for Flate/zlib, raw deflate, LZW, RunLength, ASCIIHex, ASCII85, ASCIIHex -> Flate chains, PNG predictors, and TIFF predictors.
- Streaming decompression cap enforcement is unit-tested with a small test cap.

Accuracy regression slices:

- Field slice: unchanged, `macro_field_f1 = 0.72503`, `macro_value_f1 = 0.81434`, precision `0.69231`, recall `0.84536`.
- Table slice independent re-run: unchanged, `macro_shape_f1 = 0.96232`, `macro_cell_f1 = 0.98737`, `macro_cell_recall = 0.99689`, `macro_cell_precision = 0.98246`, `macro_teds_approx = 0.98111`.
- 200-file text slice, wellfriendpdf-only: unchanged, `char_similarity = 0.92743`, `word_f1 = 1.0`, `line_recall = 1.0`, `spurious_line_ratio = 0.07633`, `reading_order = 0.96019`.

Fuzz and malformed checks:

- `cargo +nightly fuzz run filters -- -runs=256`: passed.
- `cargo +nightly fuzz run predictor -- -runs=256`: passed.
- Synthetic Flate bomb, render path: clean error, `I/O error: FlateDecode output exceeds decompression cap`, 5.9 MiB peak WS, no memory-cap hit.
- Synthetic corrupt Flate, render path: clean error, `I/O error: corrupt deflate stream`, 8.8 MiB peak WS, no memory-cap hit.
- Note: the text extraction probe treats page-level content failures as skippable and can emit empty text for a bad page; render was used for the process-level clean-error robustness assertion.

## Remaining Gaps

1. Image filters are still outside this content-tokenizer pass. DCT/JPX/CCITT/JBIG2 remain in the image pipeline and should be audited separately for full-buffer hot spots.
2. Encrypted streams and streams stored inside object streams still use the existing object-resolution path before content decoding. That path is correct, but it is not the raw range-reader path measured here.
3. The 4.0 GiB compressed target is synthetic. It proves the memory behavior for large Flate content streams, but it should be repeated on a real 3-4 GB customer PDF when one is available.
