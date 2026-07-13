# Prompt 21 Bindings

Prompt 21 reports are exposed through the shared SDK facade and bindings.

| Binding | Surface |
| --- | --- |
| Rust | `oxide_engine::prompt21_*` functions and `prompt21` module types |
| CLI | `prompt21-report`, `raster-vector-report`, `font-reconstruction-report`, `history-report`, `object-stream-report`, `save-object-streams` |
| Python | `PyDocument.prompt21_*` methods and module `prompt21_history_report()` |
| C ABI | `oxide_document_prompt21_*` functions |
| WASM | `prompt21*Json` methods and `prompt21PackObjectStreams()` |
| .NET | `OxideDocument.Prompt21*` methods |
| Java | `Oxide.Document.prompt21*` methods |

No binding silently enables external glyph generation or host path access. WASM operates on caller-provided bytes only.
