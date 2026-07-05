# Prompt 03 Codec Isolation Design

## Scope

Prompt 03 isolates the narrow lossless filter path first. These filters are centralized in `filters.rs` and can cross a process boundary as `(filter name, encoded bytes, limits) -> decoded bytes/report` without moving PDF object ownership or renderer state.

## Components

- Parent API: `crates/engine/src/codec_isolation.rs`.
- Worker binary: `crates/engine/src/bin/oxide-codec-worker.rs`.
- CLI report: `oxide codec-isolation-report`.
- Shared SDK envelope: `sdk::codec_isolation_report_json`.
- Binding surfaces: Python `oxide.codec_isolation_report`, C ABI `oxide_codec_isolation_report_json`, WASM `OxidePdf.codecIsolationReportJson`, .NET `OxideDocument.CodecIsolationReportJson`, Java `Oxide.codecIsolationReportJson`.

## Protocol

- Request/response JSON protocol version: `1`.
- Parent writes a temp request file containing request ID, codec kind, encoded bytes, decoded/dimension caps, timeout, deterministic flag, and optional trace ID.
- Worker writes a temp response file containing request ID, status, decoded byte length, decoded bytes, warnings, errors, limit failure, worker version, and elapsed time.
- Parent validates protocol version, request ID, decoded length, decoded cap, worker exit status, response JSON shape, response size cap, and timeout.

## Limits

- Default input cap: 64 MiB.
- Default decoded output cap: engine decode limit.
- Parent response JSON cap: bounded by decoded cap plus overhead, clamped to a hard response cap.
- Timeout default: 2000 ms.
- Dimension reports reuse engine width/height/pixel/decoded-byte limits.

## Failure Modes Tested

- Successful isolated Flate decode.
- Malformed input failure.
- Missing worker fail-closed behavior.
- Explicit fallback reporting for `isolated_preferred`.
- Report-only and disabled policies.
- Worker non-zero exit and deterministic crash simulation.
- Worker timeout and kill/wait containment.
- Malformed worker response JSON.
- Wrong request ID.
- Worker output-size cap.
- Unsupported codec.
- Parent input cap.
- Decoded output cap.
- Dimension cap.
- Concurrent requests.

## Platform Notes

Windows, Linux, and macOS are eligible for subprocess isolation when a worker binary is present. WASM/browser targets are report-only for subprocess policy because they cannot spawn OS processes.
