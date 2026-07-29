# README benchmark methodology

The README uses task-specific evidence instead of one combined leaderboard.

## Rules

1. Use `measured_directly` only when Wellfriend and a comparator ran a comparable operation on the same host and input.
2. Use repository validation artifacts for Wellfriend release claims.
3. Use official documentation for competitor capabilities that were not benchmarked.
4. Treat unavailable tools as unavailable, not as failures or wins.
5. Disclose wrappers: pypdfium2/PDFium, PyMuPDF/MuPDF, and pikepdf/qpdf are not independent engines.
6. Avoid cross-purpose rankings. A renderer, validator, signer, table extractor, OCR workflow, and source-linked editor are not interchangeable systems.

## Host and budget

- VPS: `35.185.176.47`
- README comparator result folder: `/home/demisuga01/wellpdf/results/readme-competitor-20260729T175541Z`
- Prompt 36 result folder: `/home/demisuga01/wellpdf/results/prompt36-20260729T063834Z`
- Memory budget for validation work: 32 GiB aggregate

## README smoke corpus

The direct README comparison used a compact repository-owned fixture normalized through pikepdf for qpdf-compatible structural checks. This made qpdf, Poppler, PDFium, MuPDF, pikepdf, pdfplumber, and Wellfriend operate on the same bytes for the direct smoke. It does not represent malformed robustness, rendering quality, OCR accuracy, accessibility review, commercial SDK behavior, or broad corpus performance.

## Prompt 36 evidence

Prompt 36 evidence is used for release posture, workspace validation, fuzz/sanitizer posture, package/binding status, memory budget, historical gate impact, and known limits. Raw logs and caches remain ignored; tracked docs summarize the relevant results.
