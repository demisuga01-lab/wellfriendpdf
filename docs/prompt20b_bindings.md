# Prompt 20B bindings

Prompt 20B is exposed through the shared SDK surface rather than separate
language-specific editing engines.

- Rust: `prompt20b_report_json`, `prompt20b_text_range_analyze_json`, and
  `prompt20b_text_range_edit_json`.
- CLI: `prompt20b-report`, `edit-text-range`, `form-instance-report`,
  `form-clone-one`, `annotation-appearance-shared-report`, and
  `annotation-appearance-clone-one`.
- Python: `prompt20b_report`, `prompt20b_text_range_analyze`, and
  `edit_text_range`.
- C ABI: `wellfriendpdf_document_prompt20b_report_json`,
  `wellfriendpdf_document_prompt20b_text_range_analyze_json`, and
  `wellfriendpdf_document_prompt20b_text_range_edit_json`.
- WASM: `prompt20bReportJson`, `prompt20bTextRangeAnalyzeJson`, and
  `editTextRange`.
- .NET and Java expose equivalent owned JSON and owned output-byte surfaces.

All reports use the versioned schema
`prompt20b.multirun-form-appearance-closure.v1`. Outputs are owned by the
callee surface and must be freed/disposed according to that binding's existing
memory rules. Password input remains byte/string based and is not logged by the
Prompt 20B wrappers.
