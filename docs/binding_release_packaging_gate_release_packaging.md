# Release Packaging Binding Release Packaging Gate

`scripts/release_packaging_release_gate.ps1` is the local release-style gate for Release Packaging. It builds host-available artifacts, runs package-oriented smokes, and writes evidence to `target/release_packaging-packaging-codec-isolation/`.

## Command

```powershell
powershell -ExecutionPolicy Bypass -File scripts\release_packaging_release_gate.ps1
```

Use `-ContinueOnFailure` only when collecting diagnostics from a partially configured host.

## Gate Behavior

- Required Rust/C/CLI/WASM checks fail the gate when commands fail.
- Optional package ecosystems are marked `unavailable` when their toolchain is missing:
  Python requires `python -m maturin`; .NET requires `dotnet`; Java requires `java` and `javac`.
- WASM package generation is required after Wasm Packaging. The gate calls
  `scripts/wasm_packaging_wasm_pack_gate.ps1`, which bootstraps target-local
  `wasm-pack 0.13.1` when a global binary is absent.
- Optional Java Maven/Gradle package smoke reuses the Java Packaging/02C scripts so Maven and Gradle artifact inspection remains authoritative.
- The gate writes step logs, an examples matrix, a codec threat matrix, a release manifest, an isolation smoke report, and an unavailable/failure report.

## Artifacts Checked

- Rust crate package contents and examples.
- CLI binary `wellfriendpdf`.
- Codec worker binary `wellfriendpdf-codec-worker`.
- C ABI dynamic/static library and public header.
- Python wheel when maturin is available.
- WASM target build, wasm-pack web package, wasm-pack Node package, package
  contents inspection, and packaged Node smoke.
- .NET test and NuGet package when dotnet is available.
- Java Maven and Gradle JAR package smokes when Java tooling is available.

## Policy

The manifest distinguishes package success from source-tree success. A source-tree smoke is not treated as package readiness unless the package artifact was also built or the manifest records the exact host/tooling reason it was unavailable.

Release Packaging originally recorded the wasm-pack package artifact as unavailable
because `wasm-pack` was not on PATH. Wasm Packaging closes that caveat; WASM package
readiness now requires the generated wasm-pack artifacts and Node smoke evidence.
