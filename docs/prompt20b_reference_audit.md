# Prompt 20B reference audit

`scripts/prompt20b_closure_audit.py` creates deterministic fixtures for
multi-run text range edits, nested Form clone-one, shared `/AP /R` and `/AP /D`,
appearance-state dictionaries, widget checkbox/radio appearances, and nested
appearance Forms.

The harness runs the real CLI mutation paths, reopens the outputs, runs qpdf
where available, and renders edited PDFs with Wellfriend plus available Poppler,
PDFium, and MuPDF tools. Missing PDFBox is recorded as unavailable and is not
counted as passed.

Artifacts:

- `target/prompt20-advanced-editing/prompt20b-reference-results.json`
- `target/prompt20-advanced-editing/prompt20b-diff-metrics.json`
- `target/prompt20-advanced-editing/prompt20b-metamorphic-results.json`
- `target/prompt20-advanced-editing/prompt20b-html-report/index.html`

The current Prompt 20B reference audit reports zero supported-case Wellfriend
outliers, zero unclassified failures, and zero security failures for the
executed supported cases.
