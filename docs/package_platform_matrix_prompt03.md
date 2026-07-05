# Prompt 03 Package Platform Matrix

| Surface | Artifact | Prompt 03B status | Gate |
| --- | --- | --- | --- |
| Rust | crate package and examples | passed | `cargo package -p oxide-engine --allow-dirty` |
| CLI | `oxide` binary | passed | `cargo build -p oxide-cli` |
| Codec worker | `oxide-codec-worker` binary | passed | `cargo build -p oxide-engine --bin oxide-codec-worker` |
| C ABI | header and native library | passed | `cargo build -p oxide-capi` |
| Python | wheel | passed on this Windows host | `python -m maturin build ...` |
| WASM web | wasm-pack web package | passed | `scripts/prompt03b_wasm_pack_gate.ps1` |
| WASM Node | wasm-pack nodejs package and packaged smoke | passed | `scripts/prompt03b_wasm_pack_gate.ps1` |
| .NET | NuGet package | passed on this Windows host | `dotnet pack ...` |
| Java Maven | JAR smoke | passed on this Windows host | `scripts/prompt02b_java_package_smoke.ps1` |
| Java Gradle | JAR smoke | passed on this Windows host | `scripts/prompt02c_gradle_package_smoke.ps1` |

## Remaining Honest Limits

- Browser smoke is not automated in Prompt 03B; the web package is built and inspected, while the executable smoke runs in Node against the packaged `nodejs` output.
- Cross-platform Linux/macOS package artifacts are not built on this Windows host.
- TypeScript `tsc` is not claimed by Prompt 03B; generated declarations are inspected and the checked-in `oxide.d.ts` is updated for the codec isolation method.
