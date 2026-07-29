# writer history Bindings

writer history reports are exposed through the shared SDK facade and bindings.

| Binding | Surface |
| --- | --- |
| Rust | `wellfriendpdf_engine::writer_history_*` functions and `writer_history` module types |
| CLI | `writer_history-report`, `raster-vector-report`, `font-reconstruction-report`, `history-report`, `object-stream-report`, `save-object-streams` |
| Python | `PyDocument.writer_history_*` methods and module `writer_history_history_report()` |
| C ABI | `wellfriendpdf_document_writer_history_*` functions |
| WASM | `writer_history*Json` methods and `writer_historyPackObjectStreams()` |
| .NET | `WellfriendDocument.WriterHistory*` methods |
| Java | `WellfriendPdf.Document.writer_history*` methods |

No binding silently enables external glyph generation or host path access. WASM operates on caller-provided bytes only.
