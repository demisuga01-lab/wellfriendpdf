# Combined Prompt 02 Binding Audit

Starting checkpoint: `c5ee7e8 Complete combined prompt 01 binding core surfaces`.
The initial worktree was clean.

Prompt 02 extends the Prompt 01 shared `oxide_engine::sdk` facade into WASM,
.NET, and Java without creating a second engine. WASM calls the Rust facade
directly. .NET and Java call the stable C ABI facade functions, preserving the
same JSON envelope and native ownership rules.

## Files Audited

- Shared facade: `crates/engine/src/sdk.rs`
- C ABI: `crates/oxide-capi/src/lib.rs`, `crates/oxide-capi/include/oxide.h`
- WASM: `crates/oxide-wasm/src/lib.rs`
- .NET: `bindings/dotnet/Oxide.Sdk`
- Java: `bindings/java`
- Prompt 01 matrix: `target/prompt01-binding-core/binding-gap-matrix.json`

## Implementation Summary

- WASM now exposes facade-backed report methods for security, risky content,
  parser, color, validation, forms, annotations, page operations, interactive
  content, signatures, fonts, semantic text, semantic document, and chunks.
- WASM output methods now return owned bytes plus report JSON for sanitize,
  canonicalize, and redact-terms workflows.
- .NET and Java expose C ABI-backed reports for security, parser, color,
  validation, forms, annotations, pages, interactive content, and chunks.
- .NET and Java expose sanitize, canonicalize, and redact-terms outputs with
  explicit buffer ownership and native release.
- Native loading now checks `OXIDE_NATIVE_LIBRARY`, local build outputs, and
  `runtimes/<rid>/native` package layouts.
- Prompt 02 smoke artifacts live under `target/prompt02-binding-parity/`.

## Honest Gaps

Prompt 02B closed the bounded leftovers from the initial Prompt 02 report.
Password open is public in WASM and now also routes through the C ABI, .NET,
and Java byte-based open paths. Progress remains `progress_not_supported` in
the shared feature report. Cancellation remains unsupported for the Prompt 02
WASM/.NET/Java report/output bindings; the feature report names the existing
engine render internals that can observe `CancelToken`, but no binding exposes
a no-op token API. Java Maven/JAR package smoke is covered by
`scripts/prompt02b_java_package_smoke.ps1`; Prompt 02C adds real Gradle
support through `bindings/java/build.gradle` and
`scripts/prompt02c_gradle_package_smoke.ps1`, including Gradle test/JAR/build,
packaged runtime smoke, and Maven/Gradle public API equivalence.

Browser/WebWorker WASM does not read host paths or write files directly. WASM
package glue must be regenerated with `wasm-pack` or `wasm-bindgen` before
publish.

The Prompt 02B closure table is `docs/bindings_prompt02b_closure_audit.md`.
The Prompt 02C Gradle closure table is
`docs/bindings_prompt02c_gradle_closure.md`.

The bounded gap list is generated in
`docs/bindings_prompt02_gap_matrix.md` and
`target/prompt02-binding-parity/binding-gap-matrix.json`.
