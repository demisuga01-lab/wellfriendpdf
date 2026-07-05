# Prompt 03 Multi-Language Examples

The example suite uses public SDK surfaces only. Existing Prompt 01/02 examples remain the broad workflow examples; Prompt 03 adds codec isolation examples and records all workflows in the release matrix generated under `target/prompt03-packaging-codec-isolation/`.

## Example Inventory

| Surface | General workflow example | Prompt 03 codec example | Package context |
| --- | --- | --- | --- |
| Rust | `crates/engine/examples/sdk_reports.rs` | `crates/engine/examples/prompt03_codec_isolation.rs` | `cargo package -p oxide-engine --allow-dirty` |
| CLI | CLI subcommands and help examples | `examples/cli/codec_isolation_report.ps1` | `target/debug/oxide codec-isolation-report ...` |
| Python | `crates/oxide-py/examples/sdk_reports.py` | `crates/oxide-py/examples/codec_isolation_report.py` | maturin wheel when available |
| C ABI | `crates/oxide-capi/examples/sdk_reports.c` | `crates/oxide-capi/examples/codec_isolation_report.c` | built `oxide_capi` header/lib |
| WASM | `crates/oxide-wasm/examples/browser` | `codec_isolation_report.mjs` | wasm-bindgen/wasm-pack package |
| .NET | `bindings/dotnet/examples/Prompt02Reports.cs` | `Prompt03CodecIsolation.cs` | `dotnet pack` artifact |
| Java | `bindings/java/examples/Prompt02Reports.java` | `Prompt03CodecIsolation.java` | Maven and Gradle JARs |

## Workflow Coverage

The public examples and docs cover open from file/bytes, password open, page count, page boxes, document status, repair diagnostics, plain text, spans/semantic output, reading order, tables, RAG chunks, JSON export, render page/DPI/image output, forms, annotations, redaction, sanitizer, active-content reporting, DOCX/PPTX/XLSX/HTML/Markdown export, editable-model reporting, security/signature/permissions reports, PDF/A/PDF/UA/PDF/X validation, canonicalization, package build, package smoke, artifact manifest, native discovery, schema version, license/readme metadata, and codec isolation policy reports.

Unsupported or partial workflows are represented as stable report envelopes rather than silent omissions. Progress and cancellation remain honestly unsupported where no engine-observable plumbing exists.

## Codec Isolation Commands

Rust:

```powershell
cargo run -p oxide-engine --example prompt03_codec_isolation -- in_process
```

CLI:

```powershell
cargo build -p oxide-cli -p oxide-engine --bin oxide-codec-worker
target\debug\oxide.exe codec-isolation-report --filter FlateDecode --sample-text "hello oxide" --policy isolated_required --worker target\debug\oxide-codec-worker.exe
```

Python:

```powershell
python crates\oxide-py\examples\codec_isolation_report.py in_process
```

C ABI:

```powershell
cargo build -p oxide-capi
cl /I crates\oxide-capi\include crates\oxide-capi\examples\codec_isolation_report.c target\debug\oxide_capi.dll.lib
```

WASM:

```powershell
cargo build -p oxide-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir crates\oxide-wasm\examples\browser\pkg target\wasm32-unknown-unknown\release\oxide_wasm.wasm
node crates\oxide-wasm\examples\browser\codec_isolation_report.mjs
```

.NET and Java examples are package-consumer snippets; run them against the NuGet/JAR outputs after the package gate creates them.
