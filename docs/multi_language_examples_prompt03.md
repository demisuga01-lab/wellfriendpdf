# Prompt 03 Multi-Language Examples

The example suite uses public SDK surfaces only. Existing Prompt 01/02 examples remain the broad workflow examples; Prompt 03 adds codec isolation examples and records all workflows in the release matrix generated under `target/prompt03-packaging-codec-isolation/`.

## Example Inventory

| Surface | General workflow example | Prompt 03 codec example | Package context |
| --- | --- | --- | --- |
| Rust | `crates/engine/examples/sdk_reports.rs` | `crates/engine/examples/prompt03_codec_isolation.rs` | `cargo package -p wellfriendpdf-engine --allow-dirty` |
| CLI | CLI subcommands and help examples | `examples/cli/codec_isolation_report.ps1` | `target/debug/wellfriendpdf codec-isolation-report ...` |
| Python | `crates/wellfriendpdf-py/examples/sdk_reports.py` | `crates/wellfriendpdf-py/examples/codec_isolation_report.py` | maturin wheel when available |
| C ABI | `crates/wellfriendpdf-capi/examples/sdk_reports.c` | `crates/wellfriendpdf-capi/examples/codec_isolation_report.c` | built `wellfriendpdf_capi` header/lib |
| WASM | `crates/wellfriendpdf-wasm/examples/browser` | `codec_isolation_report.mjs` | `scripts/prompt03b_wasm_pack_gate.ps1` wasm-pack web/Node packages |
| .NET | `bindings/dotnet/examples/Prompt02Reports.cs` | `Prompt03CodecIsolation.cs` | `dotnet pack` artifact |
| Java | `bindings/java/examples/Prompt02Reports.java` | `Prompt03CodecIsolation.java` | Maven and Gradle JARs |

## Workflow Coverage

The public examples and docs cover open from file/bytes, password open, page count, page boxes, document status, repair diagnostics, plain text, spans/semantic output, reading order, tables, RAG chunks, JSON export, render page/DPI/image output, forms, annotations, redaction, sanitizer, active-content reporting, DOCX/PPTX/XLSX/HTML/Markdown export, editable-model reporting, security/signature/permissions reports, PDF/A/PDF/UA/PDF/X validation, canonicalization, package build, package smoke, artifact manifest, native discovery, schema version, license/readme metadata, and codec isolation policy reports.

Unsupported or partial workflows are represented as stable report envelopes rather than silent omissions. Progress and cancellation remain honestly unsupported where no engine-observable plumbing exists.

## Codec Isolation Commands

Rust:

```powershell
cargo run -p wellfriendpdf-engine --example prompt03_codec_isolation -- in_process
```

CLI:

```powershell
cargo build -p wellfriendpdf-cli -p wellfriendpdf-engine --bin wellfriendpdf-codec-worker
target\debug\wellfriendpdf.exe codec-isolation-report --filter FlateDecode --sample-text "hello wellfriendpdf" --policy isolated_required --worker target\debug\wellfriendpdf-codec-worker.exe
```

Python:

```powershell
python crates\wellfriendpdf-py\examples\codec_isolation_report.py in_process
```

C ABI:

```powershell
cargo build -p wellfriendpdf-capi
cl /I crates\wellfriendpdf-capi\include crates\wellfriendpdf-capi\examples\codec_isolation_report.c target\debug\wellfriendpdf_capi.dll.lib
```

WASM:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\prompt03b_wasm_pack_gate.ps1
node scripts\prompt03b_wasm_pack_node_smoke.mjs target\prompt03-packaging-codec-isolation\wasm-pack\node-pkg crates\engine\tests\fixtures\minimal.pdf target\prompt03-packaging-codec-isolation\wasm-pack\wasm-pack-node-smoke.json
```

The Prompt 03B WASM gate imports the generated `nodejs` package directory, not
an internal build artifact. Browser smoke remains manual; the web package is
built and inspected by the same gate.

.NET and Java examples are package-consumer snippets; run them against the NuGet/JAR outputs after the package gate creates them.
