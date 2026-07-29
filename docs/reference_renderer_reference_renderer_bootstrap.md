# Reference Renderer Reference Renderer Bootstrap

Reference Renderer closes the Native Renderer reference-engine gap by making all three
external renderers available without global installation or administrator
steps.

Bootstrap entrypoint:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\reference_renderer_bootstrap_reference_renderers.ps1
```

Audit entrypoint:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\reference_renderer_multi_reference_audit.ps1
```

Artifacts:

- `target/native_renderer-renderer-native-replay/reference-tool-manifest-reference_renderer.json`
- `target/native_renderer-renderer-native-replay/multi-reference-render-results-reference_renderer.json`
- `target/native_renderer-renderer-native-replay/reference-disagreement-summary-reference_renderer.json`
- `target/native_renderer-renderer-native-replay/reference_renderer-html-report/index.html`

Tool strategies:

- Poppler is discovered through `POPPLER_PDFTOPPM` or `pdftoppm` on PATH.
- PDFium is discovered through `PDFIUM_TEST` or `pdfium_test(.exe)`. If absent,
  Reference Renderer creates `target/reference_renderer-tools/pdfium/pdfium_test.cmd`, backed by
  a target-local pypdfium2 virtual environment. The manifest records pypdfium2
  version, PDFium version, Python path, module path, and post-install hashes.
- MuPDF is discovered through `MUTOOL` or `mutool(.exe)`. If absent, Reference Renderer
  downloads MuPDF 1.27.0 from the pinned archive URL used by the Scoop extras
  manifest and verifies the archive SHA-256 before using `mutool.exe`.

Observed Reference Renderer host versions:

- Poppler: `pdftoppm version 26.02.0`.
- PDFium: pypdfium2 `4.30.0`, PDFium `126.0.6462.0`, build
  `pdfium-binaries`.
- MuPDF: `mutool version 1.27.0`.

Checksum posture:

- MuPDF archive: pre-use SHA-256 verification is required.
- PDFium/pypdfium2: no repository wheel lock existed before Reference Renderer, so the
  manifest records package/build provenance and post-install hashes. This is
  honest provenance, not a claim of pre-verified supply-chain pinning.

Normalized render posture:

- page selection is explicit;
- DPI defaults to 72 unless overridden;
- PNG output is required;
- background is opaque white where the renderer exposes the option;
- timeout defaults are recorded in the manifest;
- each invocation writes stdout/stderr/exit metadata into JSON.
