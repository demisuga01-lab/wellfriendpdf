# Prompt 26 binding surfaces

All public bindings call the canonical Rust engine. They do not return a fixed feature report in
place of validation or signing.

- Rust exposes standards profile/all validation, signing planning/execution, and post-sign
  reports through the crate root and SDK JSON helpers.
- CLI exposes `pdfa-validate`, `pdfua-validate`, `pdfx-validate`, `standards-validate`,
  `signature-plan-placeholder`, and `signature-sign` with explicit errors and JSON where
  supported by the command style.
- Python builds an ABI3 wheel and exposes standards envelopes plus `sign_pdf`.
- C ABI returns owned JSON/binary buffers with paired frees, null/malformed guards, and header
  declarations in `oxide.h`.
- WASM provides in-memory standards routes and exact unsupported capability reporting.
- .NET uses SafeHandle-backed ownership and `OXIDE_NATIVE_LIBRARY` resolution for tests.
- Java uses `AutoCloseable` native ownership and is built/tested through Maven and Gradle.

Tests cover validation envelopes, null/invalid inputs, ownership lifetimes, signing plan/report
routes, and real native-backed signing where supported.
