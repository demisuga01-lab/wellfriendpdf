# Release Packaging Package Artifact Manifest

The generated manifest lives at:

```text
target/release_packaging-packaging-codec-isolation/release-manifest.json
```

`target/` is ignored by git because these are build artifacts and local evidence. The tracked release gate script regenerates the manifest.

## Manifest Schema

- `schema_version`: manifest schema, currently `1`.
- `roadmap task`: `combined_release_packaging`.
- `head`: short git commit used for the run.
- `dirty_entries`: `git status --short` at gate start.
- `result`: `passed`, `failed`, or `passed_with_unavailable_optional`.
- `steps`: command, status, exit code, log path, and unavailable/failure reason.
- `artifacts`: artifact path, surface, existence, size, and SHA-256 when file-backed.
- `docs`: tracked Release Packaging docs expected in the release bundle.

## Required Inventory

The gate records Rust crate package output, CLI binary, codec worker binary, C ABI library/header, Python example/wheel status, WASM package status, .NET package status, Maven JAR status, Gradle JAR status, schema/report docs, and all Release Packaging example files.

If a non-WASM package ecosystem cannot be built on the current host, the entry must be `unavailable` with a concrete reason such as missing `dotnet`, `maturin`, `java`, or `javac`.

WASM is no longer allowed to be a soft unavailable artifact after Wasm Packaging.
The release gate bootstraps target-local `wasm-pack`, builds web and Node
packages, inspects the generated package contents, and records packaged Node
smoke evidence before marking the WASM package artifact passed.
