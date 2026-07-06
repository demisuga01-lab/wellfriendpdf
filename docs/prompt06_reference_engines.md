# Prompt 06 Reference Engines

Reference engine status is recorded in the Prompt 06 report
`target/prompt06-renderer-native-replay/reference-availability.json` and the
Prompt 06B tool manifest
`target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json`.

Current adapters:

- Poppler: uses `pdftoppm`, captures binary path, version command, stdout,
  stderr, exit status, timeout, output PNG path, and SHA-256.
- PDFium: Prompt 06B first looks for `PDFIUM_TEST` or `pdfium_test(.exe)`.
  When absent, `scripts/prompt06b_bootstrap_reference_renderers.ps1` creates a
  target-local pypdfium2-backed `pdfium_test.cmd` wrapper under
  `target/prompt06b-tools/pdfium/`. The wrapper records pypdfium2, PDFium, and
  Python provenance plus post-install hashes.
- MuPDF: Prompt 06B first looks for `MUTOOL` or `mutool(.exe)`. When absent,
  the bootstrap downloads the pinned MuPDF 1.27.0 Windows archive into
  `target/prompt06b-tools/mupdf/` and verifies the archive SHA-256 from the
  Scoop extras manifest before using `mutool.exe`.

Rendering options are normalized where practical: page number, DPI, PNG output,
opaque white background, timeout, deterministic artifact path, and captured
command. If a reference cannot execute, the page result records a concrete
failure category such as `missing_binary`, `reference_execution_failure`,
`render_timeout`, `blank_output`, or `unsupported_comparison`.

Prompt 06B closes the earlier local-host caveat: Poppler, PDFium, and MuPDF are
now represented in the multi-reference audit. The audit still does not claim
that Oxide matches every page; it classifies whether references agree, Oxide
matches one reference, dimensions differ, or the page belongs to later-owned
renderer parity work.

Prompt 07 reuses the same reference manifest for the transparency compositing
corpus. The Prompt 07 audit writes baseline and post-implementation render
results under `target/prompt07-transparency-compositing/` and treats a missing
Poppler, PDFium, or MuPDF adapter as a blocker rather than an unavailable-tool
excuse. The audit records reference-vs-reference disagreement before assigning
an Oxide owner.
