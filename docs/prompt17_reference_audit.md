# Prompt 17 reference and metamorphic audit

The audit driver renders generated appearances plus non-axis source, redacted, and unsupported-format outputs through Oxide, target-local Poppler, PDFium, and MuPDF; qpdf checks rewritten structure. Rows are classified as secure rewrite, secure removal, expected fail-closed, all references agree/Oxide passes, reference disagreement, Oxide outlier, or unclassified failure.

Metamorphic checks cover byte-stable XFDF export, PDF-to-XFDF-to-PDF and XFDF-to-PDF-to-XFDF semantic stability, AP generation determinism, generated-vs-flattened AP visibility, sanitizer idempotence/rescan, rotated page-coordinate normalization, shared-resource isolation, and repeated non-axis output determinism.

Machine-readable results are in `target/prompt17-annotation-xfdf-media-redaction/`; the HTML entry point is `prompt17-html-report/index.html`. Release gates require zero unclassified failures, zero security-proof failures, zero overlay-only success claims, and zero Oxide outliers for supported visual rows.
