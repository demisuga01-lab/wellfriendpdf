# Real 5,044-PDF benchmark

This benchmark was run on the validation VPS using real public arXiv PDFs downloaded by URL. The PDFs were not generated fixtures. Python was used for orchestration and wrapper tools only.

## Corpus

| Field | Value |
|---|---:|
| PDF files | 5,044 |
| PDF magic verified | 5,044 |
| Duplicate SHA-256 values | 0 |
| Total bytes | 17,059,245,901 |
| Minimum file size | 48,500 bytes |
| Maximum file size | 87,230,322 bytes |
| qpdf page-count successes | 5,036 |
| qpdf page-count failures | 8 |
| qpdf-counted pages | 116,784 |

VPS result folder: `/home/demisuga01/wellpdf/results/real-5000-comparator-20260730T082710Z`

Committed aggregate: `benchmarks/results/real-5000/real-5000-aggregate.json`

## Measured rows

| Tool | Operation | Runs | Successes | Failures | Median ms | P95 ms |
|---|---|---:|---:|---:|---:|---:|
| Wellfriend | extract_text | 5,044 | 5,042 | 2 | 63.57 | 163.71 |
| Wellfriend | parse_json | 5,044 | 5,042 | 2 | 113.71 | 364.17 |
| Wellfriend | render_first_page_72 | 5,044 | 5,041 | 3 | 163.80 | 514.41 |
| qpdf | structural_check | 5,044 | 4,911 | 133 | 63.86 | 614.88 |
| Poppler | extract_text | 5,044 | 5,044 | 0 | 63.87 | 264.09 |
| Poppler | render_first_page_72 | 5,044 | 5,044 | 0 | 113.80 | 164.20 |
| MuPDF | extract_text | 5,044 | 5,041 | 3 | 163.99 | 514.13 |
| MuPDF | render_first_page_72 | 5,044 | 5,042 | 2 | 31.46 | 63.59 |
| pikepdf | open_pages | 5,044 | 5,044 | 0 | 1.75 | 4.69 |
| PyMuPDF | open_extract_first_page | 5,044 | 5,044 | 0 | 6.05 | 12.93 |
| pypdfium2 | open_render_first_page | 5,044 | 5,044 | 0 | 7.07 | 23.57 |
| pdfplumber | extract_first_page | 5,044 | 5,044 | 0 | 91.08 | 173.41 |
| PDF.js | open_first_page | 5,044 | 5,044 | 0 | 4.65 | 30.57 |
| PDFBox | open_pages | 5,044 | 5,044 | 0 | n/a | n/a |
| pdfcpu | validate | 5,044 | 5,041 | 3 | 15.50 | 113.62 |
| veraPDF | pdfa_validate_directory | 5,044 | 0 | 5,044 | n/a | n/a |
| pyHanko | signature_fields | 5,044 | 5,043 | 1 | 16.66 | 39.57 |

veraPDF was run as a PDF/A validator over the directory. The arXiv corpus is not a PDF/A conformance corpus, so the non-compliant result is recorded as measured standards-tool evidence, not as a parser crash claim.

## Limits

- The corpus is real and large, but it is arXiv-heavy and should not be treated as representative of every enterprise, scanned, government, form-heavy, or malformed PDF population.
- Wrapper tools are named as wrappers: pypdfium2 uses PDFium; PyMuPDF uses MuPDF; pikepdf uses qpdf.
- Failures are retained as measured outcomes. They were not removed from denominators.
- Raw PDFs, raw logs, per-file sample JSONL, and full validator output are retained on the VPS and are not committed.
