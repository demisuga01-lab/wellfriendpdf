# Stream Decode Foundation

This document is the Prompt 02 inventory and design record for Oxide's PDF
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
| Filter chain depth | 16 filters | `decode_stream_parts`, `decode_stream_reader_with_cap` |
| Predictor row bytes | 64 MiB | buffered and streaming predictor paths |
| Image pixels | `OXIDE_MAX_DECODE_PIXELS` or engine default | `ensure_decode_budget` before pixel sink allocation |
| Image decoded byte addressability | platform `usize` | `ensure_decode_budget` |
| Object stream cache | bounded by reader cache budget | `BoundedObjectStreamCache` in `reader.rs` |

The stream cap is an engine backstop. Server/API layers can apply tighter
request-level limits.

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

Prompt 02 did not add a general decoded-stream cache. The existing parser already
has a bounded object-stream cache (`BoundedObjectStreamCache`) for repeated object
stream member access, and rendering has its own glyph/shading caches. A general
decoded-content/image cache would need per-entry byte accounting, a max-entry
size, thread-safe eviction, and a document-level memory budget. Most content
streams are decoded once, while image pixels are often too large to cache safely.

The accepted architecture is: keep the object-stream cache, avoid caching huge
pixels, and add a future small-stream LRU only after workload evidence shows
meaningful reuse.

## Parallel Decode

Prompt 02 does not add a new global decode scheduler. Oxide already parallelizes
page-level extraction with Rayon and keeps `PdfReader` shareable across workers.
The stream decoders are deterministic, per-stream `Read` chains and are safe to
use concurrently through independent calls. Adding a second lower-level scheduler
inside the decoder would risk nested oversubscription and aggregate memory-budget
violations.

The next safe integration point is page/resource scheduling: decode independent
page content streams or image resources in bounded windows, with per-document
memory tokens. That belongs with the page/render/OCR scheduler rather than inside
individual filter implementations.

## SIMD and Delimiter Scanning

Prompt 02 does not add SIMD scanning. Prompt 01 established scalar repair and
parser-report scanning for structural markers. Stream decoding itself does not
currently depend on large delimiter scans except through parser repair. A future
SIMD implementation should accelerate only a scalar candidate scanner and must
prove exact candidate equality for `obj`, `endobj`, `stream`, `endstream`, `xref`,
`trailer`, and `startxref` on binary data with false positives.

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

## Known Limits

- Decode diagnostics still use `OxideError` strings rather than a dedicated
  public `DecodeDiagnostic` type. Parser-report does not force all streams to
  decode, so decode metrics are not yet a full-document audit histogram.
- JPX/JBIG2/CCITT are pure Rust library adapters, not subprocess-sandboxed
  codecs. Resource limits are enforced at Oxide boundaries, but there is no
  process isolation.
- The general decoded-stream cache is intentionally not implemented until
  workload data shows reuse worth the added memory complexity.
- Parallel decode is available through existing page-level parallelism, not a
  new decode-specific work-stealing scheduler.
