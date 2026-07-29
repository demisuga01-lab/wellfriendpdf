# Current capabilities

Wellfriend PDF SDK is implementation-complete for the true-editing roadmap at commit `d346915de5125fccf3163847cb3ebec197c49046`, with release posture `release_ready_with_limits`.

## Verified with limits

- Source provenance over bytes, revisions, COS objects, content streams, instructions, display items, scene nodes, semantic nodes, and transactions.
- Canonical parser and writer path with save/reopen validation.
- Operator-preserving editing for resolved text, path, image, and Form XObject occurrences.
- Editable scene graph, transactions, inverse operations, undo reports, font reconstruction, Unicode extraction, shaping, and subset rebuilding.
- Geometric and semantic reflow with explicit edit modes, line layout, overflow policy, constraints, confidence thresholds, and undo.
- Tables, mathematical content, OCR layers, annotations, forms, XFA preservation, and cross-system integration under documented boundaries.
- Accessibility repair, tagged-PDF handling, redaction, sanitization, residual checks, security posture, standards/signature reporting, and release evidence.
- Rust, CLI, Python, C ABI, WASM, .NET, Java Maven, and server surfaces.

## Typed refusal boundaries

Wellfriend should refuse instead of silently applying when source mapping, confidence, signatures, structure repair, font/shaping, OCR quality, viewer appearance parity, XFA dynamics, standards obligations, or resource limits are outside supported policy.

## External limits

qpdf and Poppler were available in Prompt 36. README smoke additionally measured pypdfium2/PDFium, PyMuPDF/MuPDF, pikepdf/qpdf, pdfplumber, and PDFBox for narrow operations. veraPDF, pdfcpu, OCRmyPDF, and standalone mutool were unavailable on the README VPS and are not counted as losses.
