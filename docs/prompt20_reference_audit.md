# Prompt 20 reference audit

Executable Rust fixtures prove saved RTL and Identity-V text extraction, exact
old-text absence, same-width incremental prefix preservation, vector range
isolation, deterministic ink fitting, and fitted annotation appearance
readback. `scripts/prompt20_reference_harness.py` also generates a deterministic
mixed text/vector/Ink PDF, performs same-width, RTL, vertical, vector, and Ink
mutations through the public CLI, and renders the source plus five edited PDFs.

On this Windows audit host Poppler and qpdf are installed; the established
target-local PDFium wrapper and MuPDF binary are available. All six PDFs pass
qpdf and render with Oxide, Poppler, PDFium, and MuPDF. All eighteen
Oxide/reference comparisons are within tolerance, with zero supported-case
Oxide outliers and zero unclassified failures. PDFBox remains unavailable and
is not counted as a pass.

Canonical artifacts are under `target/prompt20-advanced-editing/`, including
reference results, disagreements, diff metrics, metamorphic results, and the
HTML report. Supported-case Oxide outliers, unclassified failures, and security
failures are separate counters.
