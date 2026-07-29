# roadmap closure 02 Binding Audit

Starting checkpoint: `c5ee7e8 Complete roadmap closure 01 binding core surfaces`.
The initial worktree was clean.

Binding Parity extends the Binding Surface shared `wellfriendpdf_engine::sdk` facade into WASM,
.NET, and Java without creating a second engine. WASM calls the Rust facade
directly. .NET and Java call the stable C ABI facade functions, preserving the
same JSON envelope and native ownership rules.

## Files Audited

- Shared facade: `crates/engine/src/sdk.rs`
- C ABI: `crates/wellfriendpdf-capi/src/lib.rs`, `crates/wellfriendpdf-capi/include/wellfriendpdf.h`
- WASM: `crates/wellfriendpdf-wasm/src/lib.rs`
- .NET: `bindings/dotnet/WellfriendPdf`
- Java: `bindings/java`
- Binding Surface matrix: `target/binding_surface-binding-core/binding-gap-matrix.json`

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
- Native loading now checks `WELLFRIENDPDF_NATIVE_LIBRARY`, local build outputs, and
  `runtimes/<rid>/native` package layouts.
- Binding Parity smoke artifacts live under `target/binding_parity-binding-parity/`.

## Honest Gaps

Java Packaging closed the bounded leftovers from the initial Binding Parity report.
Password open is public in WASM and now also routes through the C ABI, .NET,
and Java byte-based open paths. Progress remains `progress_not_supported` in
the shared feature report. Cancellation remains unsupported for the Binding Parity
WASM/.NET/Java report/output bindings; the feature report names the existing
engine render internals that can observe `CancelToken`, but no binding exposes
a no-op token API. Java Maven/JAR package smoke is covered by
`scripts/java_packaging_java_package_smoke.ps1`; Gradle Packaging adds real Gradle
support through `bindings/java/build.gradle` and
`scripts/gradle_packaging_gradle_package_smoke.ps1`, including Gradle test/JAR/build,
packaged runtime smoke, and Maven/Gradle public API equivalence.

Browser/WebWorker WASM does not read host paths or write files directly. WASM
package glue must be regenerated with `wasm-pack` or `wasm-bindgen` before
publish.

The Java Packaging closure table is `docs/bindings_java_packaging_closure_audit.md`.
The Gradle Packaging Gradle closure table is
`docs/bindings_gradle_packaging_gradle_closure.md`.

The bounded gap list is generated in
`docs/bindings_binding_parity_gap_matrix.md` and
`target/binding_parity-binding-parity/binding-gap-matrix.json`.
