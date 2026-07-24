# Cross-Binding Diagnostics Prompt 02

Prompt 02 preserves diagnostics by routing public binding methods through the
shared Rust facade or the stable C ABI facade functions. Binding layers are not
allowed to reinterpret report schemas or drop warnings.

## Error Taxonomy

The engine reports structured errors through `WellfriendError` and C ABI status
codes:

- `0`: success
- `2`: handled engine or input error
- `3`: panic boundary

.NET raises `WellfriendPdfException` with the native status. Java raises
`WellfriendPdf.WellfriendPdfException` with the native status. WASM maps handled errors to
`JsValue` strings and installs the panic hook when enabled.

## Report Envelope

All report methods return the shared envelope:

- `schema_version`
- `kind`
- `status`
- `warnings`
- `errors`
- report-specific payload fields

Prompt 02 does not introduce surface-specific schema forks. Convenience layers
may return bytes plus report JSON, but the report JSON itself remains the same.

## Limits, Progress, and Cancellation

Limit exceeded states are preserved as engine errors or report diagnostics.
Prompt 02B makes progress and cancellation posture machine-readable in the
shared feature report. Progress is reported as `progress_not_supported` because
the Prompt 02 report/output facade has no phase or step callback events.
Cancellation is reported as
`cancellation_not_supported_for_prompt02_bindings`: engine render internals
already have `render_page_cancellable` and
`render_display_list_cancellable_with_mode`, but WASM/.NET/Java Prompt 02
bindings do not expose token-aware render operations or token-aware facade
report/output calls. No binding exposes a fake callback, no-op abort signal, or
ignored cancellation token.

## Golden Parity

`target/prompt02-binding-parity/cross-binding-parity.json` compares SHA-256
hashes of byte-identical UTF-8 JSON reports emitted by the .NET and Java C ABI
wrappers. The compared report set is feature, security, parser, color,
validate-security, forms, annotations, pages, interactive, chunks, sanitize,
and canonicalize. The C ABI tests exercise the same exported functions
directly; WASM calls the Rust facade directly and is matrixed separately until
regenerated JS glue is available for runtime hash comparison.
