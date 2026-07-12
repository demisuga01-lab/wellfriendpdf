# Prompt 20 public surfaces and bindings

The shared Rust implementation lives in `crates/engine/src/prompt20.rs`.
Bindings do not implement an alternate editor. Rust exposes text analysis and
serialized edits, same-width analysis/apply, vector list/edit/delete/duplicate,
ink fitting and annotation appearance fitting, reports, deterministic options,
and Prompt 18B signature preflight. The CLI exposes the corresponding mutation
commands plus `prompt20-report`.

Python, C ABI, WASM, .NET, and Java expose the versioned Prompt 20 report,
vector inventory, text edits, vector edits, and Ink fitting through the shared
SDK facade. Mutations return a new owned PDF plus versioned JSON; no binding has
its own parser or writer. Java Maven and Gradle use the same Java artifact and
native C ABI. C output buffers and strings use the established explicit free
path, Python and WASM return owned language objects, and .NET/Java document
handles retain their existing disposal semantics.

The mutation entry points are `prompt20_text_edit`, `prompt20_vector_edit`, and
`prompt20_ink_fit` (idiomatically cased in .NET and Java). Text mode and option
objects, vector operations, and Ink fitting options are JSON encoded against
the schema below. Errors remain stable SDK/C-ABI errors with object, page, and
operator context where available. Password bytes are consumed only by the
shared facade and are never included in reports.

Schema: `prompt20.vertical-rtl-patch-vector-ink-editing.v1`.
