# advanced editing closeout bindings

advanced editing closeout is exposed through the shared SDK surface rather than separate
language-specific editing engines.

- Rust: `advanced_editing_closeout_report_json`, `advanced_editing_closeout_text_range_analyze_json`, and
  `advanced_editing_closeout_text_range_edit_json`.
- CLI: `advanced_editing_closeout-report`, `edit-text-range`, `form-instance-report`,
  `form-clone-one`, `annotation-appearance-shared-report`, and
  `annotation-appearance-clone-one`.
- Python: `advanced_editing_closeout_report`, `advanced_editing_closeout_text_range_analyze`, and
  `edit_text_range`.
- C ABI: `wellfriendpdf_document_advanced_editing_closeout_report_json`,
  `wellfriendpdf_document_advanced_editing_closeout_text_range_analyze_json`, and
  `wellfriendpdf_document_advanced_editing_closeout_text_range_edit_json`.
- WASM: `advanced_editing_closeoutReportJson`, `advanced_editing_closeoutTextRangeAnalyzeJson`, and
  `editTextRange`.
- .NET and Java expose equivalent owned JSON and owned output-byte surfaces.

All reports use the versioned schema
`advanced_editing_closeout.multirun-form-appearance-closure.v1`. Outputs are owned by the
callee surface and must be freed/disposed according to that binding's existing
memory rules. Password input remains byte/string based and is not logged by the
advanced editing closeout wrappers.
