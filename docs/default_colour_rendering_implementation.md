# Default colour-space rendering and colour-state correction

Status: implemented in source, **not runtime-qualified**. This increment does
not close the full editor/rendering roadmap or establish an Adobe comparison.
It extends the existing uncommitted candidate on `main`, based on
`27e62db3a1b84804339e65b6025273fd003b3736`.

No compiler, build, typecheck, test, PDF workload, renderer, browser, benchmark,
deployment, commit or push was executed for this increment. Regression functions
below describe intended assertions, not observed outcomes.

## Source changes

- `render/default_colorspace.rs` binds a selected colour space to its resource
  scope. It follows references and aliases with cancellation and a depth bound,
  honours `DefaultGray`, `DefaultRGB` and `DefaultCMYK`, treats null defaults as
  absent, and prevents resources shadowing intrinsic device names. Replacement
  spaces must have matching component counts and cannot be Lab, Indexed or
  Pattern. Their own graphs do not recursively reapply defaults.
- Default binding also traverses a Pattern base, Indexed base and the alternate
  used for Separation/DeviceN screen conversion. The result is an ephemeral
  renderer graph, never a source-object or PDF-byte rewrite. Selected graphs
  stay bound when a nested program changes resources and are restored by q/Q.
- The initial page DeviceGray selection is also bound; a document need not emit
  an explicit `g` operator to select its page default.
- Raw and packed device-colour operators, named colour selection, image
  XObjects, inline images and shading dictionaries use this shared binding.
  Explicit transparency-group colour-space declarations enter the existing
  group-policy logic through the same resolver; its existing blend-space
  limitations remain.
- Inline colour-space arrays now reach decoding. Inline abbreviations are
  canonicalised only in colour-space positions, without rewriting colourant
  names, dictionaries or named resource definitions. Packed named payloads
  cannot override an intrinsic device-space selection.
- A selected space now has its defined initial colour: CMYK starts at
  `[0, 0, 0, 1]`, Separation/DeviceN at all-one tints, and Lab/ICC zero components
  are clipped to their declared ranges. Pattern selection clears the previous
  pattern and selects no paint. A no-paint fill does not discard a simultaneous
  stroke or a pending path clip. Other missing, explicitly selected pattern
  resources still fail; no-paint is not a catch-all for malformed resources.
- SC/SCN operands are no longer prematurely clipped to `[0,1]` in the generic
  graphics state or packed dispatcher. Lab coordinates and Indexed values keep
  their original domains until resource-aware conversion. Indexed colour
  selection rounds half upward and clips to `hival`; non-finite values remain
  invalid. ICC paint and image CMM inputs use validated Range bounds. This
  preserves the existing eight-bit CMM input pipeline, not higher precision.
- Uncoloured tiling patterns retain their actual selected base colour space,
  components and bound object across an empty tile resource dictionary. Base
  colour is no longer guessed as Gray/RGB/CMYK from component count. Fill and
  stroke plate telemetry select the matching caller colour.
- Normalised display-list paths do not bake device pixels when defaults exist.
  Type 3 geometry with explicit colours similarly routes through resource-bound
  character-procedure replay; geometry can still supply clipping where supported.
  If that replay fails, it does not fall back to uncalibrated paint. Ordinary
  glyph painting does not create a text clip merely because full replay ran.
- Default resource chains participate in tile dependencies. Bound image graphs
  participate in existing decode identities. This is deliberately conservative
  invalidation; it does not establish measured cache or latency performance.
- Indexed palette streams now share bounded lossless decoding for path colour
  and image conversion, including direct and indirect streams. Encoded bytes
  are not interpreted as palette entries. `hival` is bounded to 255; the supported
  final palette is at most 4096 bytes, with bounded filter intermediates.
- SVG/PostScript classification routes scopes containing defaults to the
  existing native whole-page raster fallback, with an explicit reason. Strict
  vector export retains an explicit refusal. This is a fidelity safeguard, not
  native vector preservation of default colour-space graphs.

Main integration files: `content/state.rs`, `render/display_list.rs`,
`render/page_renderer.rs`, `render/colorspace.rs`, `render/cmm.rs`,
`images/decoder.rs`, `render/vector_fallback.rs`, and `render/mod.rs`.

## Unexecuted regression source

Twenty-six new functions are in:

- `render/default_colorspace_tests.rs`: nine graph, initial-value, alias,
  cancellation, malformed-space, inline-normalisation and palette-decode cases.
- `render/default_color_render_tests.rs`: seventeen raw/packed/image/shading,
  resource-scope, Pattern, Type 3, initial-colour, non-unit-component, tile-cache
  and vector-routing cases. Pixel assertions and a generated PDF fixture are
  present in source; none was rendered or opened during this implementation.

Three existing regressions were revised: initial Pattern no-paint and fractional
Indexed rounding are valid; an uncoloured pattern without a declared base still
fails rather than inferring a base from the operand count.

`rustfmt` parsed the touched Rust files and `git diff --check` checked tracked
whitespace. These checks do not resolve Rust names or types, check ownership,
link bindings, execute assertions or validate pixels.

## Standards used

The decisions above follow default-space selection, colour initial values,
underlying-space treatment and Indexed selection rules in ISO 32000-1 sections
8.6.5.6, 8.6.6.3 and 8.6.8, plus the inline-image errata. The implementation
was checked against primary specification text, not inferred from Acrobat UI
behaviour:

- [ISO 32000-1 PDF specification](https://developer.adobe.com/document-services/docs/assets/35e4369068f86065372c18787171a17e/PDF_ISO_32000-1.pdf)
- [ISO 32000-2 draft, colour operators and Indexed colour spaces](https://developer.adobe.com/document-services/docs/assets/5b15559b96303194340b99820d3a70fa/PDF_ISO_32000-2.pdf)
- [PDF Association ISO 32000-2 graphics errata](https://pdf-issues.pdfa.org/32000-2-2020/clause08.html)

## Still open

- All exact-revision compilation, regression execution, cross-binding execution,
  independent colour/render comparison, real-PDF and performance qualification.
- General non-RGB/ICC transparency compositing, special annotation-group colour
  scope rules, soft-mask/overprint/device simulation and arbitrary Type 3
  clipping/compositing are not completed by binding a group colour space.
- Native SVG/PostScript default-space graph preservation. The raster routing
  above must remain visible in export reports and vector-only policy.
- Arbitrary ICC profile models, higher-precision samples and exact source-domain
  versus remapped-domain treatment throughout all image Decode/palette routes.
  In particular, source ICC palette ranges and device-to-ICC default replacement
  need distinct provenance if palette values are rescaled. This change must not
  be advertised as a complete colour-management implementation.
- The subsequent `icc_alternate_conversion_implementation.md` increment replaces
  ICC image channel-count guessing with a shared metadata-validated alternate
  path for paint, images and vector shading, and retains Indexed palette backend
  policy. Its regressions are unexecuted. The earlier image sample-domain/Decode
  provenance and precision gaps above remain open.
- Large resource graphs, hostile compressed palettes, nested scope combinations
  and repeated Type 3 colour-resource lookup need runtime budget/performance
  evidence. Bounds and cancellation are implementation controls, not benchmarks.
- General layout, tagged-object migration, rich-text/structural collaboration,
  full vertical typography, scan reconstruction, production UI integration and
  the remaining work in `universal_editor_roadmap_tracking.md` stay open.

This is a renderer correctness increment. It is not universal editing,
production readiness, standards certification or evidence of Adobe superiority.
