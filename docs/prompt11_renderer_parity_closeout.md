# Prompt 11 Renderer Parity Close-Out

Prompt 11 aggregates renderer parity evidence from Prompt 06 through Prompt 10F
and writes a close-out bundle under
`target/prompt11-renderer-cmm-closeout/`.

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

- `renderer-closeout-corpus-manifest-prompt11.json`
- `renderer-closeout-render-results-prompt11.json`
- `renderer-closeout-diff-metrics-prompt11.json`
- `renderer-closeout-reference-disagreements-prompt11.json`
- `renderer-closeout-fallback-taxonomy-prompt11.json`
- `renderer-closeout-performance-memory-prompt11.json`
- `renderer-closeout-html-report/index.html`

## Close-Out Rule

Renderer parity can be called closed only when Wellfriend outlier failures and
unclassified failures are zero, or the final Prompt 11 status is partial. Every
unsupported row must name an exact feature and later owner.
