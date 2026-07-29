# Report Schema Versioning Binding Parity

Binding Parity keeps the Binding Surface envelope contract. Bindings may add language
convenience wrappers, but the JSON report string returned for an operation must
stay a shared facade envelope.

## Stable Fields

Every report envelope must include:

- `schema_version`
- `kind`
- `status`
- `warnings`
- `errors`

Report-specific payload fields are owned by the Rust facade. Bindings should
not rename them, hide warnings, or convert unsupported states into successful
empty payloads.

## Surface Rules

- Rust and WASM call `wellfriendpdf_engine::sdk` directly.
- Python and C ABI retain the Binding Surface facade path.
- .NET and Java call C ABI functions and return JSON strings unchanged.
- Output operations return `{ bytes, reportJson }` or equivalent records; the
  `reportJson` value is still the shared envelope.

## Version Queries

- WASM: `WellfriendPdf.sdkVersion()`, `WellfriendPdf.abiVersion()`
- .NET: `WellfriendDocument.EngineVersion()`, `WellfriendDocument.AbiVersion`
- Java: `WellfriendPdf.engineVersion()`, `WellfriendPdf.abiVersion()`
- C ABI: `wellfriendpdf_version()`, `wellfriendpdf_abi_version()`

An envelope-shape change must bump the report envelope version and update the
gap matrix, docs, and parity fixtures in the same change.

## Java Packaging Diagnostic Fields

Java Packaging adds progress/cancellation posture inside the `feature_report`
payload without changing the envelope shape or envelope version:

- `report.progress.status = "progress_not_supported"`
- `report.cancellation.status =
  "cancellation_not_supported_for_binding_parity_bindings"`
- `report.cancellation.engine_observable_operations` names existing engine
  render internals that can observe `CancelToken`

Bindings return this JSON unchanged. They must not expose convenience progress
or cancellation APIs until the shared facade accepts callbacks or tokens that
the engine actually observes.
