# Report Schema Versioning Prompt 02

Prompt 02 keeps the Prompt 01 envelope contract. Bindings may add language
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

- Rust and WASM call `oxide_engine::sdk` directly.
- Python and C ABI retain the Prompt 01 facade path.
- .NET and Java call C ABI functions and return JSON strings unchanged.
- Output operations return `{ bytes, reportJson }` or equivalent records; the
  `reportJson` value is still the shared envelope.

## Version Queries

- WASM: `OxidePdf.sdkVersion()`, `OxidePdf.abiVersion()`
- .NET: `OxideDocument.EngineVersion()`, `OxideDocument.AbiVersion`
- Java: `Oxide.engineVersion()`, `Oxide.abiVersion()`
- C ABI: `oxide_version()`, `oxide_abi_version()`

An envelope-shape change must bump the report envelope version and update the
gap matrix, docs, and parity fixtures in the same change.

## Prompt 02B Diagnostic Fields

Prompt 02B adds progress/cancellation posture inside the `feature_report`
payload without changing the envelope shape or envelope version:

- `report.progress.status = "progress_not_supported"`
- `report.cancellation.status =
  "cancellation_not_supported_for_prompt02_bindings"`
- `report.cancellation.engine_observable_operations` names existing engine
  render internals that can observe `CancelToken`

Bindings return this JSON unchanged. They must not expose convenience progress
or cancellation APIs until the shared facade accepts callbacks or tokens that
the engine actually observes.
