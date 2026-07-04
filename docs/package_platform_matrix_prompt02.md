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
| Gradle metadata/package | implemented_public | `bindings/java/build.gradle` plus `scripts/prompt02c_gradle_package_smoke.ps1` run Gradle 9.6.1 fallback, `clean test`, `jar`, and `build`. |
| Gradle JAR artifact | implemented_public | `bindings/java/build/libs/oxide-sdk-0.1.0.jar` is inspected as a ZIP and run through `PackageSmoke`. |
| Maven/Gradle equivalence | implemented_public | `target/prompt02-binding-parity/java-maven-gradle-equivalence.json` compares class lists, reflection API summaries, manifest module name, and runtime smoke results. |

Run Java with `--enable-preview --enable-native-access=ALL-UNNAMED`.
For Gradle builds, run `scripts/prompt02c_gradle_package_smoke.ps1` or the
equivalent `gradle --no-daemon -p bindings/java clean test jar build` commands
with `OXIDE_NATIVE_LIBRARY` set or `target/debug/oxide_capi` built.

## CI Smoke Expectations

Prompt 02 package CI should run:

- `cargo build -p oxide-wasm --target wasm32-unknown-unknown`
- wasm-pack or wasm-bindgen package regeneration where installed
- `.NET` build, tests, and `dotnet pack`
- Java compile/test with matching JDK preview flags
- `scripts/prompt02b_java_package_smoke.ps1` for Maven/JAR/package runtime smoke
- `scripts/prompt02c_gradle_package_smoke.ps1` for Gradle/JAR/package runtime
  smoke and Maven/Gradle equivalence
- `scripts/prompt02b_memory_gate.ps1` for local handle-lifetime stress evidence
- `python scripts/gen_prompt02_binding_matrix.py`
- `python scripts/write_prompt02_smoke_artifacts.py`
