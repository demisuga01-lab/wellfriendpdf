# Competitor comparison

Directly measured on the real 5,044-PDF VPS corpus: Wellfriend Standard, qpdf, Poppler, MuPDF, pikepdf, PyMuPDF, pypdfium2/PDFium, pdfplumber, PDF.js, PDFBox, pdfcpu, veraPDF, and pyHanko. Commercial SDKs remain documentation-only because no licensed benchmark executable was available.

The large-corpus evidence is in `benchmarks/results/real-5000/real-5000-aggregate.json` and summarized in `docs/benchmarks/real-5000-results.md`. Wrapper relationships are explicit: pikepdf wraps qpdf, PyMuPDF wraps MuPDF, and pypdfium2 wraps a bundled PDFium build.
