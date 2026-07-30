# Current benchmark evidence

Benchmarks were run on the validation VPS. Compact engineering fixtures remain in `benchmarks/results/latest/` and `benchmarks/results/release-candidate/`. A real public 5,044-PDF run is committed in `benchmarks/results/real-5000/real-5000-aggregate.json`.

## Real public corpus

The real corpus contains 5,044 downloaded arXiv PDFs, 17,059,245,901 bytes, and 116,784 qpdf-counted pages across the 5,036 files where qpdf page counting succeeded. The PDFs were downloaded from public URLs and were not generated fixtures.

See `docs/benchmarks/real-5000-results.md` for the measured Wellfriend, qpdf, Poppler, MuPDF, PDFium-wrapper, PDF.js, PDFBox, pdfcpu, veraPDF, and pyHanko rows.

## Wellfriend measured tasks

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

## Corpus

- Fixture count: 4
- Total bytes: 2756
- Provenance: repository-generated fixtures plus checked-in compact scan fixture

## Limits

- These are compact engineering benchmarks, not a claim of universal performance.
- Mutating tasks include save/reopen or residual verification in the measured operation.
