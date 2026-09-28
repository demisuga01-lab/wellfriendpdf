# Optimized image provenance and Indexed sample domains

Source-only continuation on `main` over
`27e62db3a1b84804339e65b6025273fd003b3736`. Changes extend the existing dirty
roadmap candidate. No build, typecheck, tests, rendering/PDF workloads, benchmark,
deployment, commit or push was performed.

## Optimized paths

The raw-window and scaled-JPEG decoder entry points now accept the original
colour-space graph separately from the effective rendering graph. Canonical
XObject rendering resolves the original alias in the selecting page/Form scope;
inline rendering passes the already resolved original graph. This provenance
reaches the same sample-domain conversion used by full-image decoding. The
existing source-aware image cache identity remains in use.

The internal call sites, soft-mask caller and eleven existing regression call
sites were adapted to the extra argument. The soft-mask path supplies no override
and retains its original dictionary. Standalone inline resolved-space and JPEG
entry points now retain their supplied original space too.

An ICC conversion without a separate source graph treats its supplied graph as
the original space. An unresolved original alias is no longer silently assumed
to have a unit Decode domain. An explicit Decode array does not require unused
original defaults, but its type, component count, references and endpoints must
still validate. Renderer paths supply provenance when performing remapping.

## Indexed image conversion

`images/indexed_samples.rs` is the common palette and index conversion route.
The canonical raw builder uses row-aware packed samples; the byte-converter
compatibility entry recovers previously normalized integer levels and uses the
same Decode/palette implementation.

- Indexed pixel Decode now applies before index selection. Default indices retain
  their integer sample values; descending, constant and non-unit Decode maps
  round to the nearest index with half-up handling and clipping to `hival`.
- Lookup bytes are decoded once with the existing bounded stream decoder. Their
  values are scaled into the **original base space's** component ranges, not
  automatically the replacement space's ranges. This includes signed Lab and
  non-unit ICC ranges.
- Those component values are converted through the effective rendering base.
  ICC palette conversion prepares the profile once for the whole palette and
  preserves the requested backend policy. No-paint bases retain transparent
  alpha rather than becoming opaque black.
- Source/target component counts and palette extent must agree. Forbidden
  Indexed/Pattern bases, malformed dimensions/bit depths, invalid Decode values,
  incorrect lookup lengths and unexpected converted-palette shapes fail with
  errors. Output allocations are bounded and cancellation is checked before,
  during and after conversion.
- Source dictionaries/lookup streams are not mutated. Existing Gray/RGB/RGBA
  channel conventions are retained by base family; ICC may produce RGB from a
  profile or RGBA through its alternate.

The superseded inline palette converter and dead raw-builder Lab/ICC/Indexed
branches were removed. The converted-palette length guard remains on the active
path. Its tests still exercise that guard; the obsolete low-bit index helper
test now calls the actual decoder rather than a retired helper.

## Regression source and checks

Eighteen new unexecuted regression functions cover packed/normalized Indexed
agreement, pixel Decode, fractional clipping/rounding, source-vs-remapped palette
domains, Lab/ICC ranges, filtered lookup, no-paint, invalid metadata, budgets,
cancellation, full-inline integration, optimized raw windows/scaled JPEG and
unresolved-source handling. An earlier scoped cache regression now also compares
full and raw-window output using separate cache keys so it cannot merely hit the
previous full-image cache entry. One earlier low-bit regression was rewritten to
exercise the shared production route.

`rustfmt` parsed the changed source; tracked and new-file whitespace was inspected.
These checks do not resolve Rust names, types or ownership and do not execute any
assertion. The new JPEG fixture exists only in regression source and was not
encoded or decoded during this implementation.

## Standards basis

Palette bytes map to the ranges of the corresponding base-space components;
pixel Decode and palette interpretation are separate operations. The algorithm
follows [ISO 32000-1 section 8.6.6.3 and image Decode table 90](https://developer.adobe.com/document-services/docs/assets/35e4369068f86065372c18787171a17e/PDF_ISO_32000-1.pdf).

## Remaining roadmap work

- Follow-up: `indexed_paint_domain_implementation.md` replaces the separate
  non-image palette helper and carries original domains through selected paint
  state and pattern tiles. Shading/export/prepress consumers still require
  separate integration; neither increment closes all colour paths.
- CCITT/JBIG2 one-component colour remapping, JPX colour interpretation and
  component-selected raw exports need further integration. This increment closes
  the specified raw-window/scaled-JPEG provenance paths, not every codec path.
- Native CMM still quantizes to its existing eight-bit input interface. Wider
  source reading/alternate conversion is not high-precision native ICC support.
- Full ICC program coverage, blending/overprint/soft-mask and native vector colour
  boundaries remain. Resource-graph/cache overhead still needs measurement.
- All current build, binding, independent rendering/extraction, corpus and load
  qualification remains unrun. No universal, production-ready or Adobe-superior
  claim follows from these source changes.
- The complete editing/layout/tag/font/collaboration/scan/UI roadmap remains
  active and is not narrowed by this report.
