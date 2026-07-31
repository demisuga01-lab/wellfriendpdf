# Full capability comparison

This table is task-specific: 72-DPI all-pages rendering on the same 5,044-real-PDF corpus. It is not an overall product ranking.

| Tool/path | Successes | Failures | Pages rendered | Median ms | P95 ms | P99 ms | Evidence |
|---|---:|---:|---:|---:|---:|---:|---|
| Wellfriend current | 5044/5044 | 0 | 116975 | 947.4 | 4170.3 | 11414.4 | `wellfriend-render-all-pages-final-current-2-summary.json` |
| Wellfriend display-list path | 5044/5044 | 0 | 116975 | 970.4 | 4269.2 | 11114.6 | `wellfriend-render-all-pages-display-list-current-summary.json` |
| pypdfium2_pdfium_wrapper | 5044/5044 | 0 | 116975 | 127.5 | 792.2 | 1621.4 | `artifacts/comparator-pypdfium2-all-72-w1/renderer-all-72dpi-summary.json` |
| pymupdf_mupdf_binding | 5044/5044 | 0 | 116975 | 1402.6 | 4265.9 | 6958.2 | `artifacts/comparator-pymupdf-all-72/renderer-all-72dpi-summary.json` |
| mupdf_mutool | 5041/5044 | 3 | 116975 | 413.1 | 1314.2 | 2604.3 | `artifacts/comparator-mupdf-all-72/renderer-all-72dpi-summary.json` |
| poppler_pdftoppm | 5044/5044 | 0 | 116975 | 2251.6 | 6172.8 | 10325.6 | `artifacts/comparator-poppler-all-72-w8/renderer-all-72dpi-summary.json` |
| apache_pdfbox | 5044/5044 | 0 | 116975 | 4186.3 | 10581.1 | 16796.8 | `artifacts/comparator-pdfbox-all-72-w8/renderer-all-72dpi-summary.json` |
| pdfjs_node_canvas | 5039/5044 | 5 | 116866 | 1857.5 | 14541.5 | 64698.3 | `artifacts/comparator-pdfjs-all-72-timeoutrows/renderer-all-72dpi-summary.json` |

Wrapper relationships are disclosed: pypdfium2 wraps PDFium; PyMuPDF wraps MuPDF. MuPDF `mutool`, Poppler `pdftoppm`, PDFBox, and PDF.js are separate execution paths. Wellfriend's display-list path was measured separately and retained for replay diagnostics; it was not faster than Wellfriend's immediate path on this all-pages corpus.
