# Prompt 36 Binding Release Matrix

Binding validation covered Rust, CLI, Python wheel build/install smoke, C ABI,
WASM check/package, .NET test/pack, and Java Maven test/package.

Evidence:

- `target/prompt36-enterprise-validation/binding-release-matrix.json`
- `target/prompt36-enterprise-validation/api-stability-report.json`

Gradle is classified as a VPS host-tool limit because the installed Gradle 4.4.1
cannot evaluate the repository's modern settings file. Maven validated Java
runtime/package behavior.
