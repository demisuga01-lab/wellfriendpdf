# Prompt 06B Reference Renderer Bootstrap

Prompt 06B closes the Prompt 06 reference-engine gap by making all three
external renderers available without global installation or administrator
steps.

Bootstrap entrypoint:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\prompt06b_bootstrap_reference_renderers.ps1
```

Audit entrypoint:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\prompt06b_multi_reference_audit.ps1
```

Artifacts:

- `target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json`
- `target/prompt06-renderer-native-replay/multi-reference-render-results-prompt06b.json`
- `target/prompt06-renderer-native-replay/reference-disagreement-summary-prompt06b.json`
- `target/prompt06-renderer-native-replay/prompt06b-html-report/index.html`

Tool strategies:

- Poppler is discovered through `POPPLER_PDFTOPPM` or `pdftoppm` on PATH.
- PDFium is discovered through `PDFIUM_TEST` or `pdfium_test(.exe)`. If absent,
  Prompt 06B creates `target/prompt06b-tools/pdfium/pdfium_test.cmd`, backed by
  a target-local pypdfium2 virtual environment. The manifest records pypdfium2
  version, PDFium version, Python path, module path, and post-install hashes.
- MuPDF is discovered through `MUTOOL` or `mutool(.exe)`. If absent, Prompt 06B
  downloads MuPDF 1.27.0 from the pinned archive URL used by the Scoop extras
  manifest and verifies the archive SHA-256 before using `mutool.exe`.

Observed Prompt 06B host versions:

- Poppler: `pdftoppm version 26.02.0`.
- PDFium: pypdfium2 `4.30.0`, PDFium `126.0.6462.0`, build
  `pdfium-binaries`.
- MuPDF: `mutool version 1.27.0`.

Checksum posture:

- MuPDF archive: pre-use SHA-256 verification is required.
- PDFium/pypdfium2: no repository wheel lock existed before Prompt 06B, so the
  manifest records package/build provenance and post-install hashes. This is
  honest provenance, not a claim of pre-verified supply-chain pinning.

Normalized render posture:

- page selection is explicit;
- DPI defaults to 72 unless overridden;
- PNG output is required;
- background is opaque white where the renderer exposes the option;
- timeout defaults are recorded in the manifest;
- each invocation writes stdout/stderr/exit metadata into JSON.
