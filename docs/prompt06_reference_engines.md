# Prompt 06 Reference Engines

Reference engine status is recorded in
`target/prompt06-renderer-native-replay/reference-availability.json`.

Current adapters:

- Poppler: uses `pdftoppm`, captures binary path, version command, stdout,
  stderr, exit status, timeout, output PNG path, and SHA-256.
- PDFium: looks for `PDFIUM_TEST` or `pdfium_test(.exe)`. Missing binaries are
  recorded as unavailable, not passed.
- MuPDF: looks for `MUTOOL` or `mutool(.exe)`. Missing binaries are recorded as
  unavailable, not passed.

Rendering options are normalized where practical: page number, DPI, PNG output,
timeout, deterministic artifact path, and captured command. If a reference
cannot execute, the page result records a concrete failure category such as
`missing_binary`, `reference_execution_failure`, `render_timeout`,
`blank_output`, or `unsupported_comparison`.

The Prompt 06 local audit found Poppler available. PDFium and MuPDF were not
available in this environment and therefore have closure actions instead of
false pass results. Prompt 07 can begin with Poppler evidence, but PDFium/MuPDF
closure remains required for release-grade multi-reference parity.
