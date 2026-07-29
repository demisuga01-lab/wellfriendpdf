# Renderer Fuzz CMM Renderer Parity Close-Out

Renderer Fuzz CMM aggregates renderer parity evidence from Native Renderer through Porterduff Radial Color Glyph
and writes a close-out bundle under
`target/renderer_fuzz_cmm-renderer-cmm-closeout/`.

## Reference Policy

The close-out bundle tracks Poppler, PDFium, MuPDF, and WellfriendPdf. If a reference
renderer disagrees with another reference renderer, Wellfriend is classified relative
to the reference cluster rather than treated as automatically wrong.

Classifications are:

- all-reference Wellfriend pass
- reference-disagreement Wellfriend-inside-cluster
- reference-disagreement Wellfriend-outside-cluster
- unsupported-reported expected
- malformed/reference failure
- Wellfriend outlier
- unclassified failure

## Required Artifacts

- `renderer-closeout-corpus-manifest-renderer_fuzz_cmm.json`
- `renderer-closeout-render-results-renderer_fuzz_cmm.json`
- `renderer-closeout-diff-metrics-renderer_fuzz_cmm.json`
- `renderer-closeout-reference-disagreements-renderer_fuzz_cmm.json`
- `renderer-closeout-fallback-taxonomy-renderer_fuzz_cmm.json`
- `renderer-closeout-performance-memory-renderer_fuzz_cmm.json`
- `renderer-closeout-html-report/index.html`

## Close-Out Rule

Renderer parity can be called closed only when Wellfriend outlier failures and
unclassified failures are zero, or the final Renderer Fuzz CMM status is partial. Every
unsupported row must name an exact feature and later owner.
