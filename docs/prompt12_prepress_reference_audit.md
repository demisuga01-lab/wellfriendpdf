# Prompt 12 Prepress Reference Audit

Prompt 12B runs the prepress visual corpus through:

- Wellfriend default/fallback
- Wellfriend native LittleCMS where the native feature gate is enabled in validation
- Poppler
- PDFium
- MuPDF

The target-local renderer tools are reused from the Prompt 06B reference harness:

- Poppler `pdftoppm`
- PDFium wrapper under `target/prompt06b-tools/pdfium`
- MuPDF `mutool` under `target/prompt06b-tools/mupdf`

The Prompt 12B audit corpus covers Separation text/vector, stencil-image plate
sampling, Separation shading, tiling pattern plate sampling, DeviceN component
handling, BPC/rendering-intent report posture, and native/fallback ICC report
posture.

Artifacts:

- `target/prompt12-prepress-cmm/prepress-reference-tool-manifest-prompt12b.json`
- `target/prompt12-prepress-cmm/prepress-reference-render-results-prompt12b.json`
- `target/prompt12-prepress-cmm/prepress-reference-diff-metrics-prompt12b.json`
- `target/prompt12-prepress-cmm/prepress-reference-disagreement-summary-prompt12b.json`
- `target/prompt12-prepress-cmm/prompt12b-html-report/index.html`

Spot and DeviceN visual preview disagreements are classified separately from
Wellfriend internal plate data. External reference renderers generally expose
flattened previews rather than true plate framebuffers, so the pass condition is
zero Wellfriend outliers where references agree and zero unclassified failures.
