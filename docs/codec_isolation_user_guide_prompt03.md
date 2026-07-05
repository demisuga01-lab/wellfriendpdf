# Prompt 03 Codec Isolation User Guide

## Policies

- `in_process`: default behavior; decode in the parent with existing limits.
- `isolated_preferred`: try the worker; fall back in-process only if policy permits and report the fallback.
- `isolated_required`: use the worker or fail closed.
- `report_only`: return a diagnostic report without decoding.
- `disabled`: report disabled by policy.

## CLI

```powershell
cargo build -p oxide-cli -p oxide-engine --bin oxide-codec-worker
target\debug\oxide.exe codec-isolation-report --filter FlateDecode --sample-text "hello oxide" --policy isolated_required --worker target\debug\oxide-codec-worker.exe
```

The output is a versioned JSON envelope:

- `schema_version`: report envelope version.
- `kind`: `codec_isolation_report`.
- `status`: `ok` for the SDK envelope when report generation succeeded.
- `report.status`: codec outcome such as `success`, `failed_closed`, `worker_timeout`, `fallback_success`, or `report_only`.
- `report.worker_used`: whether a subprocess worker was used.
- `report.fallback_used`: whether in-process fallback occurred by explicit policy.

## Bindings

All bindings expose the same report shape:

- Rust: `oxide_engine::decode_filter_with_isolation`.
- Python: `oxide.codec_isolation_report(filter, data, policy="in_process")`.
- C ABI: `oxide_codec_isolation_report_json`.
- WASM: `OxidePdf.codecIsolationReportJson(filter, bytes, policy)`.
- .NET: `OxideDocument.CodecIsolationReportJson(filter, bytes, policy)`.
- Java: `Oxide.codecIsolationReportJson(filter, bytes, policy)`.

## Deployment Guidance

Ship `oxide-codec-worker` beside `oxide` or set `OXIDE_CODEC_WORKER` to the worker path. Use `isolated_required` for hostile customer PDFs when fail-closed behavior is acceptable. Use `isolated_preferred` only when an explicit in-process fallback is acceptable to the product.

Do not describe this as a full sandbox. It is crash, timeout, and output-size containment.

## Prompt 04 Additions

Prompt 04 adds an enforceable native/C codec boundary on top of the Prompt 03 subprocess worker:

- default builds remain pure Rust;
- native/C codec dependencies are denied by default;
- future native backends must be represented in the central codec registry;
- the native dependency allowlist is empty unless explicitly extended;
- native backends must be feature-gated and worker/sandbox-required;
- `CodecIsolationReport` now includes `backend_selection` and `native_boundary` fields.

The feature report also exposes Prompt 04 scanner, renderer scheduler, and RLBox/WASM posture under `report.prompt04`. The SDK envelope version remains `1`.
