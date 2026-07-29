# roadmap closure 04 Codec Boundary And Scheduler Audit

## Starting Checkpoint

- Starting HEAD: `79b15aa`.
- Starting commit: `79b15aa Close roadmap closure 03B wasm-pack packaging gate`.
- Starting worktree status: clean (`git status --short` produced no entries).
- Baseline commands run before edits:
  - `git status --short`
  - `git rev-parse --short HEAD`
  - `git log --oneline -n 25`

## Current Codec Worker Architecture

- Parent/worker implementation lives in `crates/engine/src/codec_isolation.rs`.
- Worker binary lives in `crates/engine/src/bin/wellfriendpdf-codec-worker.rs`.
- Current worker protocol version is `CODEC_WORKER_PROTOCOL_VERSION = 1`.
- Current policies are `in_process`, `isolated_preferred`, `isolated_required`, `report_only`, and `disabled`.
- Current worker-supported filters are `FlateDecode`, `ASCIIHexDecode`, `ASCII85Decode`, `RunLengthDecode`, and `LZWDecode`.
- Parent-side controls already include input cap, decoded output cap, width/height/pixel caps, timeout, request ID validation, response JSON size cap, worker exit handling, malformed response handling, and fail-closed `isolated_required`.
- `isolated_preferred` explicitly reports fallback; `isolated_required` refuses in-process fallback.
- Current gap: worker-supported codec names and native-codec policy are not represented as a central backend registry with implementation-language, native-dependency, feature-flag, sandbox, allowlist, and platform metadata.

## Current Filter Implementations

- Shared stream filter decoding lives in `crates/engine/src/filters.rs`.
- Implemented lossless stream filters include Flate, LZW, ASCIIHex, ASCII85, and RunLength.
- Image-only filters (`DCTDecode`, `JPXDecode`, `CCITTFaxDecode`, `JBIG2Decode`) are intentionally stopped in the stream layer and decoded by image modules.
- Image decoders live under `crates/engine/src/images/`; current codec crates are Rust dependencies (`jpeg-decoder`, `hayro-jpeg2000`, `hayro-ccitt`, `hayro-jbig2`).
- `crates/engine/src/lib.rs` has `#![forbid(unsafe_code)]`, so Codec Boundary acceleration and policy enforcement must remain safe in the engine crate.

## Current Parser Scanner Paths

- `crates/engine/src/decode_scanner.rs` already defines `PDF_DELIMITER_MARKERS`, `MarkerCandidate`, `MarkerScanResult`, scalar scanning, and an accelerated entry point.
- Current accelerated entry point returns `ScalarFallbackNoUnsafeSimd` and delegates directly to scalar scanning.
- Parser/reader marker searches still use ad hoc safe `windows(...).position()`/`rposition()` scans in `parser.rs` and `reader.rs` for `endstream`, `stream`, `xref`, and `startxref`.
- Current gap: the accelerated scanner abstraction is not wired into parser marker searches and does not provide independent safe acceleration.

## Current Renderer Image Decode Paths

- Immediate renderer is `crates/engine/src/render/page_renderer.rs`.
- XObject image decode calls `ImageDecoder::decode` directly from `handle_do_image`.
- Inline image decode calls `ImageDecoder::decode_inline` directly from `paint_inline_image`.
- Soft masks call `SmaskLoader::load_and_combine`, which calls `ImageDecoder::decode` directly.
- Form XObject streams, SMask group streams, annotation appearance streams, pattern streams, and shading streams call `crate::filters::decode_stream` directly.
- Tile and band render paths render the full page deterministically, then crop into tiles/bands; tile caching uses `RenderCache`.
- Current gap: render-time image and some stream decode paths do not acquire scheduler memory tokens or expose renderer scheduler metrics.

## Current Scheduler APIs

- `crates/engine/src/decode_scheduler.rs` defines:
  - `DecodeMemoryBudget`
  - `DecodeMemoryToken`
  - `DecodeSchedulerMetrics`
  - `ScheduledDecodeJob`
  - `run_scheduled_decode_jobs`
- Existing scheduler tests prove deterministic result ordering and budget rejection for scheduled jobs.
- Current scheduler is batch-oriented and can run jobs through Rayon with deterministic result sorting.
- Current gap: the renderer does not own a per-render decode scheduler context and does not route direct image/stream decode calls through memory token acquisition.

## Current Memory Budget APIs

- `DecodeLimits` in `filters.rs` contains:
  - per-stream decoded cap
  - per-document decoded cap
  - image dimension/pixel/decoded-byte caps
  - codec-specific JPX/JBIG2/CCITT/DCT caps
  - `max_concurrent_decode_jobs`
  - `scheduler_memory_budget_bytes`
  - cache budget fields
- `images::decoder::ensure_decode_budget` checks image dimensions before allocating pixel buffers.
- Renderer page-size caps are enforced before `PixelBuffer` allocation.
- Current gap: renderer decode paths use the image dimension checks, but not scheduler memory reservations around each decode.

## Current Binding Report Fields

- Shared SDK report facade lives in `crates/engine/src/sdk.rs`.
- Report envelope version is `REPORT_ENVELOPE_VERSION = 1`.
- `sdk::codec_isolation_report_json` wraps `CodecIsolationReport`.
- `sdk::feature_report_json` includes `codec_isolation_availability_report`.
- Python, C ABI, WASM, .NET, and Java surfaces call the shared SDK facade or native C ABI report entry points; current tests assert envelope shape and representative fields.
- Current gap: codec reports do not expose native-boundary registry metadata, scanner implementation posture, RLBox/WASM feasibility posture, or renderer scheduler adoption posture.

## Codec Boundary Implementation Bounds

- This roadmap task must not implement renderer parity features such as transparency parity, shadings, patterns, font raster parity, color glyphs, or PDFium/MuPDF comparison.
- This roadmap task must not add native/C codec dependencies by default.
- The practical implementation path is:
  - add enforceable registry/policy metadata beside `codec_isolation`;
  - hard-block or prototype RLBox/WASM with command-backed evidence;
  - replace scanner fallback with safe accelerated candidate discovery and wire marker lookups through it;
  - add a per-render decode scheduler context with memory token acquisition and deterministic synchronous execution for current immediate renderer decode paths;
  - surface the new posture through existing SDK reports and docs without changing the report envelope version.

## Validation Note

- A 1 GiB process-tree RAM cap was too low for rebuild-heavy validation on this host: `rustc`, `wasm-pack`, Python extension rebuild, and the Gradle test JVM could exceed that cap.
- Validation was rerun with a 4 GiB process-tree RAM cap using a Windows Job Object, `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, and `RUST_TEST_THREADS=1`.
- Under the 4 GiB cap, `cargo test --workspace --jobs 1 --quiet` passed.
- Under the 4 GiB cap, `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1 --quiet` passed.
- Under the 4 GiB cap, `cargo check --manifest-path fuzz/Cargo.toml --bins --jobs 1 --quiet` passed.
- Under the 4 GiB cap, `cargo build -p wellfriendpdf-py --jobs 1 --quiet` passed, and `python -m pytest crates/wellfriendpdf-py/tests/test_reports.py -q` passed against the freshly built extension copied to an ignored temporary import directory.
- Under the 4 GiB cap, `scripts/release_packaging_release_gate.ps1` passed, including cargo package/build, codec isolation CLI/tests, Python wheel build, .NET test/pack, Java Maven/Gradle smokes, and wasm-pack packaging.
