# Competitor comparison

This comparison is task-specific. It does not create an overall winner score, and it does not count unavailable tools as failures.

## Same-host measured rows

| Tool | Task | Operation | Median ms | P95 ms | Status |
|---|---|---|---:|---:|---|
| qpdf | qpdf_check | structural_check | 1.689 | 1.795 | measured_comparable |
| qpdf | qpdf_rewrite | structural_rewrite | 1.791 | 1.822 | measured_comparable |
| Poppler pdfinfo | poppler_pdfinfo | page_count | 3.858 | 3.966 | measured_comparable |
| Poppler pdftotext | poppler_pdftotext | text_extraction | 3.975 | 4.172 | measured_comparable |
| pikepdf (qpdf wrapper) | pikepdf_open_save | open_save | 0.347 | 0.380 | measured_comparable |
| pypdfium2 (PDFium wrapper) | pypdfium2_page_count | page_count | 0.038 | 0.051 | measured_comparable |
| PyMuPDF (MuPDF wrapper) | pymupdf_text_render | text_and_render | 0.513 | 0.590 | measured_comparable |
| pdfplumber | pdfplumber_text | text_extraction | 0.668 | 0.724 | measured_comparable |

## Documentation-only or unavailable landscape

| Tool | Scope | Evidence class |
|---|---|---|
| Adobe PDF Library / Datalogics | commercial SDK; documentation-only | official_competitor_documentation |
| Apryse | commercial SDK; documentation-only | official_competitor_documentation |
| Nutrient | commercial SDK; documentation-only | official_competitor_documentation |
| Foxit PDF SDK | commercial SDK; documentation-only | official_competitor_documentation |
| iText | commercial SDK/library; documentation-only | official_competitor_documentation |
| veraPDF | standards validator; unavailable in this run | unavailable_or_not_measured |
| PDFBox | Java PDF toolkit; not benchmarked in this run | unavailable_or_not_measured |
| PDF.js | browser viewer; not benchmarked in this run | unavailable_or_not_measured |
| pdfcpu | Go structural tool; not benchmarked in this run | unavailable_or_not_measured |
| Tesseract/OCRmyPDF | OCR workflow tools; not benchmarked in this run | unavailable_or_not_measured |
| Docling/Camelot | structured extraction tools; not benchmarked in this run | unavailable_or_not_measured |

## Interpretation

- qpdf and pikepdf are structural specialists, not semantic editors.
- Poppler, PDFium, MuPDF, and PDF.js are mature viewing/rendering/extraction ecosystems.
- veraPDF is a standards validator; pyHanko is a signature specialist.
- pdfplumber, Camelot, Docling, Tesseract, and OCRmyPDF are extraction/OCR oriented.
- Commercial SDKs may have proprietary behavior that is not represented by these local measurements.
- Wellfriend is positioned around provenance-linked true editing, transactions, undo, semantic reflow, and integrated document subsystems.
