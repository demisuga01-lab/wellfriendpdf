# advanced editing closeout reference audit

`scripts/advanced_editing_closeout_closure_audit.py` creates deterministic fixtures for
multi-run text range edits, nested Form clone-one, shared `/AP /R` and `/AP /D`,
appearance-state dictionaries, widget checkbox/radio appearances, and nested
appearance Forms.

The harness runs the real CLI mutation paths, reopens the outputs, runs qpdf
where available, and renders edited PDFs with Wellfriend plus available Poppler,
PDFium, and MuPDF tools. Missing PDFBox is recorded as unavailable and is not
counted as passed.

Artifacts:

- `target/advanced_editing-advanced-editing/advanced_editing_closeout-reference-results.json`
- `target/advanced_editing-advanced-editing/advanced_editing_closeout-diff-metrics.json`
- `target/advanced_editing-advanced-editing/advanced_editing_closeout-metamorphic-results.json`
- `target/advanced_editing-advanced-editing/advanced_editing_closeout-html-report/index.html`

The current advanced editing closeout reference audit reports zero supported-case Wellfriend
outliers, zero unclassified failures, and zero security failures for the
executed supported cases.
