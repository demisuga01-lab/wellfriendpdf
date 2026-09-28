# Indexed paint domains and selected colour provenance

Source-only continuation on `main`, based on
`27e62db3a1b84804339e65b6025273fd003b3736`, including the existing uncommitted
roadmap implementation. This increment is not a completed roadmap or a qualified
release. No compiler, build, test, PDF workload, renderer execution, benchmark,
binding execution or deployment was run.

## Implemented source changes

- Non-image Indexed paint now uses the same palette preparation and original
  base-component mapping as Indexed image decoding. The old separate palette
  range/component helpers in `render/colorspace.rs` were removed. Paint converts
  only its selected entry and retains floating-point output and alpha; it does
  not round through an image pixel buffer. Image palettes retain their batched
  ICC preparation and existing output channel conventions.
- Selected fill/stroke colour state retains both the scope-resolved original
  graph and the Default*-remapped rendering graph. Raw and retained-plan
  selection, q/Q, inherited Form replay and active-state snapshots carry both.
  A missing active binding is resolved from the actual resource scope rather
  than silently treating a remapped graph as original input.
  Immutable `Arc` handles share selected graphs and ICC stream bytes through
  selected-state lookup and saved-state cloning without adding another pair of
  graph copies at that step. Palette/ICC preparation still resolves and copies
  metadata, and other consumers still allocate; no latency/RSS improvement is
  claimed without measurement.
- Uncoloured tiling patterns carry both underlying base graphs into the tile
  resource scope. Fill, stroke and forced retained vector paint use the retained
  binding, even when the tile has empty/different resources.
- Type 3 inherited-paint cache identity includes the original graph as well as
  the target graph. Equal target resources are not sufficient when their source
  palette domains differ.
- Named-colour conversion propagates corresponding original alternate graphs
  through ICC, Separation and DeviceN conversion. A Default* replacement whose
  family differs from its source is treated as a new subtree, not matched to an
  unrelated source alternate. ICC image-domain conversion accepts the same
  provenance, including when a palette occurs inside an ICC alternate.
- The named resolver polls cancellation and enforces a stack-scoped depth limit
  of 32. Existing ICC depth, bounded lookup decoding and image budgets remain.
  Unresolvable alternate references are rejected rather than retried as if
  their metadata were usable. Invalid Indexed metadata remains a typed invalid
  colour rather than opaque default black.

## Regression source, not executed evidence

Eleven new regression functions were added:

- Six in `images/indexed_samples_tests.rs`: image/paint domain agreement and
  unquantized alpha/colour, nested ICC alternate provenance, Separation/DeviceN
  CIE alternate propagation, Default* replacement subtree handling, no-ink
  transparency, and malformed/nonfinite input rejection.
- Four in `render/default_color_render_tests.rs`: fill/stroke selection across
  scope changes/qQ/Form replay; differing source domains with identical target
  graphs and Type 3 inherited-paint fingerprints; retained fill/stroke/forced
  vector dispatch; and source-aware uncoloured fill/stroke pattern tiles.
- One in `render/colorspace.rs`: recursive alternate rejection and cancellation
  leave the depth guard reusable for subsequent paint.

Two existing Indexed assertions were updated for shared metadata validation:
malformed calibrated bases now report invalid Indexed metadata at preparation,
and a prohibited Pattern base reports invalid rather than unhandled.

`rustfmt` formatting/parser-only checks accepted the changed Rust source, and
`git diff --check` found no tracked whitespace errors. These do not resolve Rust
types, ownership or symbols and do not establish that any regression passes.

## Standards and boundaries

Palette interpretation follows the base-component ranges in ISO 32000-1
section 8.6.6.3; pixel Decode and palette byte interpretation are separate.
Source provenance is an internal implementation mechanism, not an added PDF
dictionary extension. Separation/DeviceN alternate examples use a CIE-based
ICC graph, not a direct special-colour alternate. See the
[Adobe-hosted ISO 32000-1 specification](https://developer.adobe.com/document-services/docs/assets/35e4369068f86065372c18787171a17e/PDF_ISO_32000-1.pdf).

Follow-up: `shading_color_domain_implementation.md` carries original domains and
selected colour policy through shading conversion. General nonlinear mesh
interpolation, opacity and aggregate raster budgeting still need implementation.
This increment does not close source provenance in every colour consumer:
native SVG/PostScript emission, prepress plate reporting,
legacy normalized codec/component-export paths and non-RGB blending have
separate integration work. Native ICC conversion still has its existing
eight-bit interface. General ICC profiles, codec coverage, overprint, soft masks,
resource/cache cost and all independent visual qualification remain open.
The source resolver is not a complete PDF colour-space conformance validator.

The full editing/layout/tag/font/history/scan/UI roadmap remains active in
`universal_editor_roadmap_tracking.md`. No universal, production-ready or
better-than-Acrobat claim follows from this increment.
