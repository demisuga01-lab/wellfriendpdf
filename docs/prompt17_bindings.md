# Prompt 17 public bindings

All bindings call `crates/engine/src/sdk.rs` and return versioned JSON envelopes. Output operations return owned bytes plus an owned report. C callers free buffers with `oxide_buffer_free` and strings with `oxide_string_free`. WASM accepts bytes/JSON only and exposes no path, network, player, or media-execution callback.

Surfaces:

- Rust: typed functions and reports in `oxide_engine::prompt17` and crate-root re-exports.
- CLI: `annotation-xfdf-export`, `annotation-xfdf-import`, `annotation-appearance-generate`, `annotation-appearance-report`, `rich-media-report`, `rich-media-sanitize`, `rich-media-flatten-poster`, `redact-image-nonaxis`, and `prompt17-report`.
- Python `Document`: `annotation_xfdf_export/import`, `annotation_appearance_generate/report`, `rich_media_report/sanitize/flatten_poster`, `nonaxis_redaction_plan`, `redact_image_nonaxis`, and `prompt17_report`.
- C ABI: versioned `oxide_document_*_json` functions with explicit input lengths and owned output/free rules; all Prompt 17 declarations are published in `crates/oxide-capi/include/oxide.h` and the header passes an MSVC C syntax check.
- WASM: corresponding camelCase report/output methods using in-memory bytes and JSON.
- .NET `OxideDocument` and Java `Oxide.Document`: idiomatic report methods and `OxideBinaryResult`/`BinaryResult` lifecycle-safe outputs.

The additive feature-report key is `prompt17_annotation_xfdf_media_nonaxis_redaction`; no existing key or envelope version changed.
