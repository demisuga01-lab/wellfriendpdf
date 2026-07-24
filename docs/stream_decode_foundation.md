# Stream Decode Foundation

This document is the Prompt 02 inventory and design record for Wellfriend's PDF
stream decoding layer. It covers the current central decoder path, filter
support, resource limits, tests, fuzzing, cache posture, and the places where
future work should extend the design without adding a second decode stack.

## Current Pipeline

The central stream path is `crates/engine/src/filters.rs`:

1. Resolve `/Filter` or `/F` as a name or name array.
2. Resolve `/DecodeParms` or `/DP` as a dictionary or array aligned with the
   filter chain.
3. Apply lossless byte filters in order.
4. Stop explicitly at image-only filters and return
   `StreamDecodeStatus::StoppedAtImageFilter`.
5. Route image-only filter bytes through `crates/engine/src/images/decoder.rs`
   and the codec adapters under `crates/engine/src/images/`.

The same central path is used by content streams, font streams, attachment
streams, object/image editing helpers, image XObjects, inline images where
possible, xref streams, and object streams. Image codecs remain separate
adapters because their outputs are pixels, not byte streams.

## Prompt 02 Hardening

Prompt 02 kept the existing architecture and tightened the safety envelope:

- Filter chain depth is capped at 16 filters before decoding starts.
- Buffered ASCIIHex, ASCII85, RunLength, and LZW output now use the same
  decoded-output cap as Flate.
- Streaming ASCIIHex and ASCII85 readers are wrapped in the existing capped
  reader.
- Predictor row length is capped at 64 MiB, with checked arithmetic for row
  length and bytes-per-pixel calculations.
- JPEG reads metadata and checks the image decode budget before full pixel
  decode. The JPEG decoder also receives an explicit max decoded-buffer size.
- JPX checks width, height, and stored channel count before full pixel decode.

These changes are intentionally additive and do not change normal decoded bytes
for valid documents under the caps.

## Resource Limits

| Limit | Default | Enforced in |
| --- | ---: | --- |
| Lossless decoded bytes per stream | 512 MiB | Flate, LZW, RunLength, ASCIIHex, ASCII85, streaming readers |
| Lossless decoded bytes per document | 2 GiB | `DecodeLimits` and scheduler/reporting surfaces |
| Filter chain depth | 16 filters | `decode_stream_parts`, `decode_stream_reader_with_cap` |
| Predictor row bytes | 64 MiB | buffered and streaming predictor paths |
| Image pixels | `WELLFRIENDPDF_MAX_DECODE_PIXELS` or engine default | `ensure_decode_budget` before pixel sink allocation |
| Image decoded byte addressability | platform `usize` | `ensure_decode_budget` |
| Object stream cache | bounded by reader cache budget | `BoundedObjectStreamCache` in `reader.rs` |
| General decode cache | 32 MiB budget, 4 MiB entry cap | `DecodeCache` utility |
| Decode scheduler memory budget | 512 MiB default | `DecodeMemoryBudget` tokens |

The stream cap is an engine backstop. Server/API layers can apply tighter
request-level limits through `DecodeLimits`. Public profiles are:

- `DecodeLimits::default()` for normal SDK use.
- `DecodeLimits::strict_low_memory()` for constrained service workers.
- `DecodeLimits::audit_generous()` for finite forensic inspection.

The CLI exposes this through `wellfriendpdf parser-report --include-decode
--decode-profile default|low-memory|audit` plus high-value overrides for stream
MiB, chain depth, image megapixels, and decode cache MiB.

## Public Decode Diagnostics

Prompt 02B adds a typed stream decode report surface in
`crates/engine/src/filters.rs`:

- `DecodeReport`
- `DecodeDiagnostic`
- `DecodeMetrics`
- `DecodeLimits`

Diagnostics include stable code, severity, source, filter name, chain index,
object id/generation when known, raw stream length, decoded bytes, limit name
and value for cap hits, predictor parameters, image dimensions, partial-output
status, and a stable human message.

`parser-report` exposes the report only when requested:

```sh
wellfriendpdf parser-report input.pdf --mode audit --json --include-decode
```

Example shape:

```json
{
  "decode": {
    "ok": false,
    "metrics": {
      "streams_seen": 1,
      "streams_failed": 1,
      "unsupported_filters": 1
    },
    "diagnostics": [
      {
        "code": "decode_unsupported_filter",
        "severity": "error",
        "source": "unknown_filter",
        "filter_name": "BogusDecode",
        "object": [7, 0]
      }
    ]
  }
}
```

Normal reports omit `decode` so existing parser-report JSON remains stable
unless callers opt into stream auditing.

## Filter Support Table

| Filter | Status | Streaming | Predictor support | Caps | Fuzz/property coverage | Parallel safe | Cache safe | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| FlateDecode / Fl | Supported | Yes | PNG/TIFF via `/Predictor` | output cap, predictor row cap | unit, streaming-vs-buffered, fuzz `filters`, fuzz `predictor` | Yes, per independent stream | Yes for small streams | Accepts zlib and raw deflate. |
| LZWDecode / LZW | Supported | Yes | PNG/TIFF via `/Predictor` | output cap, predictor row cap | unit, cap tests, fuzz `filters` | Yes, per independent stream | Yes for small streams | Supports `/EarlyChange` 0 or 1. |
| RunLengthDecode / RL | Supported | Yes | Not applicable | output cap | unit, cap tests, fuzz `filters` | Yes | Yes for small streams | EOD marker and truncated runs are tested. |
| ASCIIHexDecode / AHx | Supported | Yes | Not applicable | output cap | unit, cap tests, fuzz `filters` | Yes | Yes for small streams | Odd trailing nibble follows PDF behavior. |
| ASCII85Decode / A85 | Supported | Yes | Not applicable | output cap | unit, cap tests, fuzz `filters` | Yes | Yes for small streams | Handles `z`, partial groups, and `~>`. |
| DCTDecode / DCT | Supported as image codec | No byte-stream output; decodes to pixels | Not applicable | image budget before decode, JPEG buffer limit | image unit tests, fuzz `image_decoders` | Yes per image | Metadata only preferred; full pixels should be bounded | Uses pure Rust `jpeg-decoder`. |
| JPXDecode | Supported as image codec | No byte-stream output; decodes to pixels | Not applicable | image budget before decode | JPX fixture tests, fuzz `image_decoders` | Yes per image | Avoid caching large pixels | Uses pure Rust `hayro-jpeg2000`. |
| CCITTFaxDecode / CCF | Supported as image codec | Sink-based pixel output | Not applicable | columns x rows image budget | unit tests, oversized cap test, fuzz `image_decoders` | Yes per image | Avoid caching large pixels | Uses pure Rust `hayro-ccitt`. |
| JBIG2Decode | Supported as image codec | Sink-based pixel output | Not applicable | page/region image budget | malformed test, fuzz `image_decoders` | Yes per image with caution | Avoid caching large pixels | Uses pure Rust `hayro-jbig2`; no JBIG2 writing. |
| Crypt | Identity/no-op after reader decryption | N/A | N/A | encryption handler limits | parser/crypto tests | Depends on reader context | N/A | Non-identity without encryption context is a clean `EncryptedPdf` error. |
| Unknown filters | Clean unsupported error | N/A | N/A | no allocation beyond raw stream | fuzz `filters` | N/A | N/A | Diagnostics name the unsupported filter. |

## Decode Cache

Prompt 02B adds `DecodeCache`, a per-document LRU utility with exact byte
accounting:

- configurable total budget and max-entry size;
- no global state or cross-document leakage;
- no caching of failed decodes as successful bytes;
- oversize entries are skipped, not partially stored;
- metrics for hits, misses, evictions, current bytes, and skipped oversize
  entries.

The cache is intentionally for small decoded streams and metadata-like outputs.
Huge image pixels are not cached by default because that risks consuming the
document memory budget faster than repeated decode saves CPU.

## Parallel Decode

Prompt 02B adds `DecodeMemoryBudget`, `ScheduledDecodeJob`, and
`run_scheduled_decode_jobs`. This is a controlled work-stealing foundation over
Rayon:

- callers choose max workers through `DecodeLimits`;
- each job reserves memory tokens before executing;
- a job larger than the aggregate budget fails with a structured engine error;
- output is sorted by original job index for deterministic results;
- tests prove scheduled ASCIIHex decode matches serial decode.

The scheduler is not forced into every rendering/extraction path yet. That is a
deliberate integration boundary: page/render/OCR scheduling can adopt this
foundation without changing filter semantics or causing nested thread pools.

## SIMD and Delimiter Scanning

Prompt 02B adds `decode_scanner`:

- scalar reference scanner for `obj`, `endobj`, `stream`, `endstream`, `xref`,
  `trailer`, and `startxref`;
- an accelerated entry point that currently reports
  `ScalarFallbackNoUnsafeSimd`;
- equality tests between scalar and accelerated candidate sets;
- explicit documentation that scanner candidates are raw byte candidates, not
  parser-valid objects.

No unsafe SIMD was added because `wellfriendpdf-engine` has `#![forbid(unsafe_code)]`.
The future SIMD path must either use a safe portable implementation or stay
feature-gated outside the core engine. `scripts/scan_marker_bench.py` provides a
small benchmark harness for measuring whether scanner acceleration is worthwhile.

## Fuzz and Property Coverage

Fuzz targets in `fuzz/Cargo.toml` relevant to stream decoding:

- `filters`: Flate, LZW, ASCIIHex, ASCII85, RunLength.
- `predictor`: PNG/TIFF predictor geometry and row decoding.
- `image_decoders`: DCT, JPX, CCITT, and JBIG2 image paths.
- `content_tokenizer`: content streams and inline image tokenizer state.
- `xref_stream` and `object_stream`: parser consumers that depend on filtered
  stream decoding.

Metamorphic coverage in unit tests compares buffered and streaming filter output,
including filter chains and predictors. Cap tests cover decompression bombs,
filter-chain depth, predictor row geometry, ASCII output, RunLength output, LZW
output, and streaming ASCII output.

Prompt 02B adds:

- `scripts/run_decode_fuzz_campaign.py` for quick or long fuzz campaigns with
  per-target logs and JSON summaries under `target/fuzz-campaigns/`;
- compact seed inputs under `fuzz/seeds/`;
- `scripts/codec_corpus_runner.py` for user-provided hostile PDF/raw-codec
  corpora, using `parser-report --include-decode` for PDFs and metadata-only
  cataloging for raw codec files.

## Known Limits

- Python and C ABI do not yet expose decode diagnostics directly. The stable
  surfaces for Prompt 02B are Rust and parser-report JSON.
- Risky codecs remain in-process. This is a documented pure-Rust plus caps
  decision, not subprocess/RLBox isolation.
- The scheduler foundation is implemented, but broad render/extraction adoption
  is deferred to the subsystem scheduler prompts to avoid nested oversubscription.
- SIMD is not implemented in the core engine because unsafe code is forbidden;
  the scanner abstraction and equality tests are in place for a future safe
  implementation.
