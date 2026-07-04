# Package Platform Matrix Prompt 02

## WASM

| Environment | Status | Notes |
| --- | --- | --- |
| Browser | partial_public | Rust/WASM builds; regenerate wasm-bindgen glue before publish. |
| WebWorker | partial_public | Same byte-only API as browser; no host filesystem. |
| Node | partial_public | Use wasm-pack `--target nodejs`; caller reads files and passes bytes. |
| Native binary loading | unsupported_reported | WASM has no native dynamic library loading. |

## .NET

| Platform | Status | Notes |
| --- | --- | --- |
| Windows x64 | implemented_public | Verified locally with `oxide_capi.dll`. |
| Linux x64 | partial_public | Resolver supports `liboxide_capi.so`; package artifact not built in this run. |
| macOS arm64/x64 | partial_public | Resolver supports `liboxide_capi.dylib`; package artifact not built in this run. |
| NuGet metadata | implemented_public | csproj has package id, readme, license, tags, repository, docs. |

Native files should be packaged under `runtimes/<rid>/native`.

## Java

| Platform | Status | Notes |
| --- | --- | --- |
| Windows x64, JDK 25 | implemented_public | Verified with Java FFM preview flags. |
| Linux/macOS | partial_public | Loader supports mapped library names; not built in this run. |
| Maven metadata/package | implemented_public | `scripts/prompt02b_java_package_smoke.ps1` ran Maven 3.9.9 fallback, `mvn clean test`, and `mvn package`. |
| JAR artifact | implemented_public | `bindings/java/target/oxide-sdk-0.1.0.jar` was inspected as a ZIP and run through `PackageSmoke`. |
| Gradle | unsupported_reported | Maven is the authoritative Java package flow; Gradle is consumer-only over the Maven/JAR artifact. |

Run Java with `--enable-preview --enable-native-access=ALL-UNNAMED`.
For Gradle consumers, publish/install the Maven artifact and use normal Maven
coordinates:

```gradle
repositories { mavenLocal() }
dependencies { implementation("org.oxidepdf:oxide-sdk:0.1.0") }
```

## CI Smoke Expectations

Prompt 02 package CI should run:

- `cargo build -p oxide-wasm --target wasm32-unknown-unknown`
- wasm-pack or wasm-bindgen package regeneration where installed
- `.NET` build, tests, and `dotnet pack`
- Java compile/test with matching JDK preview flags
- `scripts/prompt02b_java_package_smoke.ps1` for Maven/JAR/package runtime smoke
- `scripts/prompt02b_memory_gate.ps1` for local handle-lifetime stress evidence
- `python scripts/gen_prompt02_binding_matrix.py`
- `python scripts/write_prompt02_smoke_artifacts.py`
