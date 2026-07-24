# Combined Prompt 03 Audit

## Starting State

- Required preflight commands were run before edits:
  - `git status --short`: clean.
  - `git rev-parse --short HEAD`: `747944b`.
  - `git log --oneline -n 30`: starts at `747944b Close combined prompt 02C gradle java packaging support`.
- Expected starting commit matched the prompt.
- Worktree was clean, so there were no user or generated dirty files to classify before implementation.

## Package Surfaces Found

- Rust SDK facade: `crates/engine/src/sdk.rs`, package metadata in `crates/engine/Cargo.toml`.
- CLI: `crates/cli`, binary name `wellfriendpdf`.
- Python: `crates/wellfriendpdf-py`, PyO3/maturin metadata in `pyproject.toml`.
- C ABI: `crates/wellfriendpdf-capi`, public header `crates/wellfriendpdf-capi/include/wellfriendpdf.h`.
- WASM: `crates/wellfriendpdf-wasm`, wasm-bindgen wrapper and browser example folder.
- .NET: `bindings/dotnet/WellfriendPdf` plus test project.
- Java Maven/Gradle: `bindings/java/pom.xml`, `bindings/java/build.gradle`, and Prompt 02B/02C package smoke scripts.

## Examples Found

- Rust examples: `getting_started.rs`, `inspect.rs`, `parse_to_markdown.rs`, `sdk_reports.rs`, `compliance.rs`, `editing.rs`, `authoring.rs`, `render_bench.rs`.
- CLI examples: existing CLI help and smoke commands plus new `examples/cli/codec_isolation_report.ps1`.
- Python examples: `crates/wellfriendpdf-py/examples/sdk_reports.py`, `local_ai_ocr_backend.py`.
- C examples: `parse_document.c`, `extract_text.c`, `sdk_reports.c`.
- WASM example: `crates/wellfriendpdf-wasm/examples/browser`.
- .NET examples: `bindings/dotnet/examples/Prompt02Reports.cs`.
- Java examples: `bindings/java/examples/Prompt02Reports.java`.

## Docs Found

- SDK and binding docs: `docs/wellfriendpdf_sdk.md`, `docs/binding_examples_prompt01.md`, `docs/bindings_prompt02_audit.md`, `docs/package_platform_matrix_prompt02.md`.
- Language docs: `docs/python_binding.md`, `docs/dotnet_binding.md`, `docs/java_binding.md`, `docs/c_abi_prompt01.md`.
- Packaging docs: `docs/packaging.md`, `docs/release.md`.
- Security/decode docs: `docs/decode_security_scorecard.md`, `docs/codec_sandboxing.md`, `docs/security_policy.md`.

## Codec Modules Found

- Stream filters and decode budgets: `crates/engine/src/filters.rs`.
- Image decode dispatch: `crates/engine/src/images/decoder.rs`.
- Codec-specific modules: `images/jpx.rs`, `images/jbig2.rs`, `images/ccitt.rs`.
- Encryption interaction: `crates/engine/src/crypto.rs` and stream open/decode call sites.

## Prior Decode Safety Reports

- Existing decode budget reporting came from Prompt 02 and is exposed through Rust/Python/C/WASM/.NET/Java feature/report surfaces.
- Existing defenses were in-process bounds and deterministic report envelopes. There was no OS subprocess isolation boundary before Prompt 03.

## Current Packaging Commands

- Rust crate/package: `cargo package -p wellfriendpdf-engine --allow-dirty`.
- CLI/native/C ABI: `cargo build -p wellfriendpdf-cli -p wellfriendpdf-capi -p wellfriendpdf-engine --bin wellfriendpdf-codec-worker`.
- Python wheel: `python -m maturin build --manifest-path crates/wellfriendpdf-py/Cargo.toml`.
- WASM: `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown`; package with `scripts/prompt03b_wasm_pack_gate.ps1`.
- .NET: `dotnet test bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj`; `dotnet pack bindings/dotnet/WellfriendPdf/WellfriendPdf.csproj`.
- Java Maven: `scripts/prompt02b_java_package_smoke.ps1`.
- Java Gradle: `scripts/prompt02c_gradle_package_smoke.ps1`.

Prompt 03 consolidates those commands in `scripts/prompt03_release_gate.ps1`.

## Prompt 03B Closure Addendum

- Prompt 03B starting HEAD: `d125e05`.
- Prompt 03B starting worktree: clean.
- Prompt 03B scope: close only the wasm-pack package artifact caveat from the
  Prompt 03 release manifest.
- Prompt 03B result: `scripts/prompt03_release_gate.ps1` now requires
  `scripts/prompt03b_wasm_pack_gate.ps1`; that script bootstraps target-local
  `wasm-pack 0.13.1`, builds web and Node package directories, inspects package
  contents, and runs packaged Node smoke.
- Evidence directory:
  `target/prompt03-packaging-codec-isolation/wasm-pack/`.
