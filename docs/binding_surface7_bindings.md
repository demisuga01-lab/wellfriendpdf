# annotation/media redaction public bindings

All bindings call `crates/engine/src/sdk.rs` and return versioned JSON envelopes. Output operations return owned bytes plus an owned report. C callers free buffers with `wellfriendpdf_buffer_free` and strings with `wellfriendpdf_string_free`. WASM accepts bytes/JSON only and exposes no path, network, player, or media-execution callback.

Surfaces:

- Rust: typed functions and reports in `wellfriendpdf_engine::annotation_media_redaction` and crate-root re-exports.
- CLI: `annotation-xfdf-export`, `annotation-xfdf-import`, `annotation-appearance-generate`, `annotation-appearance-report`, `rich-media-report`, `rich-media-sanitize`, `rich-media-flatten-poster`, `redact-image-nonaxis`, and `annotation_media_redaction-report`.
- Python `Document`: `annotation_xfdf_export/import`, `annotation_appearance_generate/report`, `rich_media_report/sanitize/flatten_poster`, `nonaxis_redaction_plan`, `redact_image_nonaxis`, and `annotation_media_redaction_report`.
- C ABI: versioned `wellfriendpdf_document_*_json` functions with explicit input lengths and owned output/free rules; all annotation/media redaction declarations are published in `crates/wellfriendpdf-capi/include/wellfriendpdf.h` and the header passes an MSVC C syntax check.
- WASM: corresponding camelCase report/output methods using in-memory bytes and JSON.
- .NET `WellfriendDocument` and Java `WellfriendPdf.Document`: idiomatic report methods and `WellfriendBinaryResult`/`BinaryResult` lifecycle-safe outputs.

The additive feature-report key is `annotation_media_redaction_annotation_xfdf_media_nonaxis_redaction`; no existing key or envelope version changed.
