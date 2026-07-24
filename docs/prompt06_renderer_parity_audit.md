# Combined Prompt 06 Renderer Parity Audit

Prompt 06 establishes the first renderer-parity campaign surface for Wellfriend after
the codec scheduler and fuzz closeout work. The implementation inspected and
uses these real modules:

- `crates/engine/src/render/display_list.rs`: display-list capture, native replay
  counters, compatibility fallback reasons, cache metrics, tile helpers.
- `crates/engine/src/render/page_renderer.rs`: immediate renderer, display-list
  replay through `RenderState`, image/form/text dispatch, annotation painting.
- `crates/engine/src/engine.rs`: page resource extraction, image/Form XObject
  subtype discovery, decode-scheduled content and image access.
- `crates/cli/src/main.rs`: `render` and Prompt 06 `render-compare` report
  command.
- `crates/engine/src/sdk.rs`: shared feature report surfaced to bindings.

The audit harness is `scripts/prompt06_renderer_parity_audit.py`. It writes all
Prompt 06 artifacts under `target/prompt06-renderer-native-replay/`:

- `corpus-manifest.json`
- `reference-availability.json`
- `parity-baseline.json`
- `parity-after-native-replay.json`
- `failure-taxonomy.json`
- `native-replay-counters.json`
- `visual-diff-summary.json`
- `report.html`

Prompt 06B adds a closure harness without changing the native replay
implementation:

- `scripts/prompt06b_bootstrap_reference_renderers.ps1`
- `scripts/prompt06b_multi_reference_audit.ps1`
- `scripts/prompt06b_render_compare.py`
- `target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json`
- `target/prompt06-renderer-native-replay/multi-reference-corpus-manifest-prompt06b.json`
- `target/prompt06-renderer-native-replay/multi-reference-render-results-prompt06b.json`
- `target/prompt06-renderer-native-replay/multi-reference-diff-metrics-prompt06b.json`
- `target/prompt06-renderer-native-replay/reference-disagreement-summary-prompt06b.json`
- `target/prompt06-renderer-native-replay/renderer-parity-taxonomy-prompt06b.json`
- `target/prompt06-renderer-native-replay/prompt06b-html-report/index.html`

The current corpus has 13 page-level entries: simple text, positioned text,
RTL placeholder text, CJK/CID text, Image XObject, generated inline image,
Form XObject, nested Form XObject, annotation appearance, tiling pattern,
shading, transparency, and malformed-renderable coverage.

Prompt 06B reuses that same 13-page corpus across Wellfriend, Poppler, PDFium, and
MuPDF. For each page it records Wellfriend-vs-Poppler, Wellfriend-vs-PDFium,
Wellfriend-vs-MuPDF, Poppler-vs-PDFium, Poppler-vs-MuPDF, and PDFium-vs-MuPDF
metrics. Pages are classified as reference agreement, Wellfriend mismatch, reference
disagreement with Wellfriend matching a specific reference, dimension mismatch,
reference-tool failure, Wellfriend render failure, or manual-review/later-owned
renderer work.

The Prompt 06B closure run rendered all 13 pages with all four renderers and
produced 78 pairwise comparisons. It classified 10 pages as
`all_references_agree_wellfriendpdf_pass` and 3 pages as
`references_disagree_wellfriendpdf_between_references`: annotation appearance, tiling
pattern, and shading. No reference-tool failure, Wellfriend render failure, or
dimension mismatch was recorded in that run.

Baseline is recorded as a Prompt 05 policy model: high-level text/image/form
content was treated as compatibility page content before this Prompt 06 native
operation layer. The post-native report is produced from the current
`render-compare` command and reference adapters.

Prompt 06 deliberately does not claim full renderer parity. It creates the
repeatable evidence path and moves common text, image, inline image, and Form
XObject replay into typed native operations while keeping later categories
measured as fallback.

Prompt 06B closes the multi-reference audit bootstrap gap. Remaining failures in
pattern, shading, transparency, soft-mask, advanced annotation, CJK/RTL raster,
and related categories stay bounded to later renderer prompts and are classified
instead of hidden.
