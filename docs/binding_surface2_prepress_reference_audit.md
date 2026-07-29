# Prepress CMM Prepress Reference Audit

Nchannel Plate Prepress runs the prepress visual corpus through:

- Wellfriend default/fallback
- Wellfriend native LittleCMS where the native feature gate is enabled in validation
- Poppler
- PDFium
- MuPDF

The target-local renderer tools are reused from the Reference Renderer reference harness:

- Poppler `pdftoppm`
- PDFium wrapper under `target/reference_renderer-tools/pdfium`
- MuPDF `mutool` under `target/reference_renderer-tools/mupdf`

The Nchannel Plate Prepress audit corpus covers Separation text/vector, stencil-image plate
sampling, Separation shading, tiling pattern plate sampling, DeviceN component
handling, BPC/rendering-intent report posture, and native/fallback ICC report
posture.

Artifacts:

- `target/prepress_cmm-prepress-cmm/prepress-reference-tool-manifest-nchannel_plate_prepress.json`
- `target/prepress_cmm-prepress-cmm/prepress-reference-render-results-nchannel_plate_prepress.json`
- `target/prepress_cmm-prepress-cmm/prepress-reference-diff-metrics-nchannel_plate_prepress.json`
- `target/prepress_cmm-prepress-cmm/prepress-reference-disagreement-summary-nchannel_plate_prepress.json`
- `target/prepress_cmm-prepress-cmm/nchannel_plate_prepress-html-report/index.html`

Spot and DeviceN visual preview disagreements are classified separately from
Wellfriend internal plate data. External reference renderers generally expose
flattened previews rather than true plate framebuffers, so the pass condition is
zero Wellfriend outliers where references agree and zero unclassified failures.
