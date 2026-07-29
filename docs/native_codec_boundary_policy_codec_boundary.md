# Codec Boundary Native/C Codec Boundary Policy

## Default Policy

Wellfriend's default codec posture is pure Rust. The engine crate remains `#![forbid(unsafe_code)]`, and no native/C codec dependency is compiled or enabled by default.

Native/C codec backends are denied unless all of the following are true:

- a central codec registry entry exists in `crates/engine/src/codec_isolation.rs`;
- the native dependency is present in the native dependency allowlist;
- the backend is behind the `native-codecs` feature;
- the backend requires a worker/sandbox boundary;
- reports expose the backend, dependency, feature, sandbox, and fallback state;
- failure to satisfy policy fails closed.

## Enforced Registry

Codec Boundary adds `CodecBackendRegistryEntry` and `CodecBackendSelectionReport`. Current registered backends are the existing Rust implementations:

- `FlateDecode` via `flate2`
- `ASCIIHexDecode`
- `ASCII85Decode`
- `RunLengthDecode`
- `LZWDecode`
- `DCTDecode` via `jpeg-decoder`
- `JPXDecode` via `hayro-jpeg2000`
- `CCITTFaxDecode` via `hayro-ccitt`
- `JBIG2Decode` via `hayro-jbig2`

The native dependency allowlist is intentionally empty in Codec Boundary. `validate_codec_registry_policy()` checks that any future native entry is feature-gated, worker-required, not enabled by default, and allowlisted.

## Runtime Behavior

`decode_filter_with_isolation()` now includes backend selection and native boundary fields in `CodecIsolationReport`. Unregistered codecs fail with `unsupported_codec`. Native backend requests fail closed unless a future entry satisfies all registry and feature requirements.

`isolated_required` still refuses in-process fallback. `isolated_preferred` can only fall back by explicit policy and must report `fallback_used` and `fallback_reason`.

## Reports And Tests

Report fields flow through:

- Rust SDK: `sdk::feature_report_json`, `sdk::codec_isolation_report_json`
- CLI/C/Python/WASM/.NET/Java surfaces that use the shared SDK/C ABI facade

Evidence:

- `crates/engine/tests/codec_isolation.rs`
- `target/codec_boundary-codec-boundary-scheduler/native-codec-boundary-report.json`

The SDK envelope version remains `1`; Codec Boundary adds inner report fields only.
