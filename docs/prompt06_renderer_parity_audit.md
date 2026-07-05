# Combined Prompt 06 Renderer Parity Audit

Prompt 06 establishes the first renderer-parity campaign surface for Oxide after
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

The current corpus has 13 page-level entries: simple text, positioned text,
RTL placeholder text, CJK/CID text, Image XObject, generated inline image,
Form XObject, nested Form XObject, annotation appearance, tiling pattern,
shading, transparency, and malformed-renderable coverage.

Baseline is recorded as a Prompt 05 policy model: high-level text/image/form
content was treated as compatibility page content before this Prompt 06 native
operation layer. The post-native report is produced from the current
`render-compare` command and reference adapters.

Prompt 06 deliberately does not claim full renderer parity. It creates the
repeatable evidence path and moves common text, image, inline image, and Form
XObject replay into typed native operations while keeping later categories
measured as fallback.
