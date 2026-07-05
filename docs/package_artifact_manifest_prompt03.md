# Prompt 03 Package Artifact Manifest

The generated manifest lives at:

```text
target/prompt03-packaging-codec-isolation/release-manifest.json
```

`target/` is ignored by git because these are build artifacts and local evidence. The tracked release gate script regenerates the manifest.

## Manifest Schema

- `schema_version`: manifest schema, currently `1`.
- `prompt`: `combined_prompt03`.
- `head`: short git commit used for the run.
- `dirty_entries`: `git status --short` at gate start.
- `result`: `passed`, `failed`, or `passed_or_unavailable_optional`.
- `steps`: command, status, exit code, log path, and unavailable/failure reason.
- `artifacts`: artifact path, surface, existence, size, and SHA-256 when file-backed.
- `docs`: tracked Prompt 03 docs expected in the release bundle.

## Required Inventory

The gate records Rust crate package output, CLI binary, codec worker binary, C ABI library/header, Python example/wheel status, WASM package status, .NET package status, Maven JAR status, Gradle JAR status, schema/report docs, and all Prompt 03 example files.

If a package ecosystem cannot be built on the current host, the entry must be `unavailable` with a concrete reason such as missing `dotnet`, `maturin`, `wasm-pack`, `java`, or `javac`.
