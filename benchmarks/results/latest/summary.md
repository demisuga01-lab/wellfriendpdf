# Current benchmark summary

Compact repository-generated fixtures measured in one release-profile process with one worker.

| Task | Category | Median ms | P95 ms | Failures |
|---|---:|---:|---:|---:|
| open_parse | core | 0.005 | 0.005 | 0 |
| page_count_model | core | 0.005 | 0.005 | 0 |
| text_extraction | core | 0.028 | 0.031 | 0 |
| render_page_png_72dpi | core | 0.137 | 0.143 | 0 |
| canonical_noop_rewrite | core | 0.018 | 0.018 | 0 |
| linearized_save | core | 0.071 | 0.074 | 0 |
| source_text_replacement_save_reopen | editing | 0.745 | 0.752 | 0 |
| paragraph_reflow_save_reopen | editing | 0.514 | 0.529 | 0 |
| table_cell_edit_save_reopen | editing | 37.161 | 43.102 | 0 |
| annotation_create_appearance_save_reopen | editing | 0.143 | 0.151 | 0 |
| form_text_create_appearance_save_reopen | editing | 0.134 | 0.141 | 0 |
| ocr_searchable_layer_save_reopen | ocr | 0.205 | 0.215 | 0 |
| redaction_residual_verification | security | 0.447 | 0.464 | 0 |
| accessibility_structure_repair | security | 0.088 | 0.094 | 0 |
