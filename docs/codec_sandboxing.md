# Codec Sandboxing and Risk Posture

PDF image codecs are untrusted-input attack surfaces. A small stream can declare
huge geometry, a malformed arithmetic-coded stream can stress CPU paths, and
symbol-dictionary codecs such as JBIG2 have a history of security-sensitive
failures. Oxide treats stream decoding as hostile by default.

## Current Codec Boundaries

| Codec | Implementation | Native code | Current boundary | Prompt 02 posture |
| --- | --- | --- | --- | --- |
| DCTDecode / JPEG | `jpeg-decoder` | No | Metadata read, image budget, decoder buffer limit | Supported with pre-decode geometry cap. |
| JPXDecode / JPEG 2000 | `hayro-jpeg2000` | No | Header parse, image budget before decode | Supported with pre-decode geometry/channel cap. |
| CCITTFaxDecode | `hayro-ccitt` | No | Decode parameters, columns x rows image budget, bounded sink | Supported with sink-level clipping. |
| JBIG2Decode | `hayro-jbig2` | No | Embedded image parse, image budget, bounded sink | Supported defensively; no JBIG2 writing or lossy symbol substitution. |

The selected codec crates are Rust dependencies and do not require a C compiler,
cmake, or platform dynamic libraries. Oxide still treats them as attack surface:
errors are converted to `OxideError`, output geometry is checked, and pixel sinks
clip to expected output length.

## Threat Model

The defensive model covers:

- decompression bombs in simple filters;
- filter-chain bombs such as ASCII85 to Flate to predictor;
- predictor geometry overflows from hostile `/Columns`, `/Colors`, and
  `/BitsPerComponent`;
- image pixel bombs from hostile `/Width`, `/Height`, embedded JPEG headers,
  JPX headers, CCITT `/Columns` and `/Rows`, or JBIG2 page dimensions;
- JPX tile/component expansion through the decoded output budget;
- CCITT malformed streams that should return an error instead of hanging;
- JBIG2 malformed or oversized embedded streams;
- non-identity `/Crypt` filters without an active encryption context.

## What Is Not Claimed

Oxide does not currently isolate codecs in a subprocess, RLBox sandbox, WASM
sandbox, seccomp profile, or OS job object. The current protection is in-process,
pure Rust decoding plus resource limits and fuzz/property tests. This is an
important safety boundary, but it is not equivalent to process isolation.

Oxide also does not write JBIG2. In particular, it does not perform lossy JBIG2
symbol substitution or any JBIG2 re-encoding that could alter document meaning.
If JBIG2 writing is ever added, it should be handled as a separate security and
correctness project.

## Future Isolation Path

A stronger future sandbox should keep this decode API shape and move only risky
codec execution behind a process or WASM boundary:

1. Keep `/Filter` resolution, DecodeParms validation, and resource-limit checks
   in Rust before invoking a codec worker.
2. Pass bounded raw bytes plus declared limits to the worker.
3. Enforce wall-clock timeout and memory cap outside the worker.
4. Return either bounded pixels/metadata or a structured codec diagnostic.
5. Keep deterministic fallback behavior for WASM builds, where threads and
   subprocesses may not exist.

That future work should not create a second filter pipeline. It should wrap the
codec adapters under `crates/engine/src/images/`.
