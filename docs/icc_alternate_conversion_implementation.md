# Shared ICC alternate conversion

Source-only continuation on `main`, based on
`27e62db3a1b84804339e65b6025273fd003b3736`. This increment extends the existing
uncommitted candidate; it is not a qualified release or completion of the roadmap.

## Implemented in source

`render/icc_conversion.rs` now owns metadata validation, profile decoding,
backend selection and alternate conversion for ICC paint and image samples.
The older image branches no longer guess Gray/RGB/CMYK solely from channel count
when a profile conversion is unavailable.

- Scope binding resolves explicit `/Alternate` graphs and materializes the
  specification's implicit DeviceGray/RGB/CMYK alternate when absent. This is
  an ephemeral rendering graph; original PDF objects are not rewritten.
- `/N`, `/Range`, alternate component count and non-Pattern requirements are
  checked before conversion. Malformed metadata is not an implicit device
  fallback. Alternate ranges include Lab's signed components and Indexed's
  index domain.
- Component values are clipped first to the ICC range and then to the alternate
  range, without rescaling at the handoff. `/Separation /None` remains no-paint
  for paint and transparent RGBA for alternate image output.
- Profile conversion uses the existing cached CMM backend. When unavailable,
  conversion follows the validated alternate, including nested ICC and tint
  spaces. Portable and native backend successes count as profile conversions.
- The explicit deterministic-fallback policy does not decode/use profile bytes.
  Under a profile-using policy, stream/filter decode errors propagate instead
  of silently becoming alternate success. An unsupported or rejected decoded
  profile still takes the declared alternate; that is not full ICC fidelity.
- Profile decoding is bounded, conversion recursion is capped, output byte
  counts are checked, and cancellation is polled around native calls and in
  image loops. A native codec call itself is not forcibly interrupted.
- Image alternate conversion uses a bounded 256-entry per-call sample cache.
  Common image conversion checks dimensions with checked arithmetic. Native
  output must contain exactly the expected number of RGB bytes.
- Named paint and vector-shading conversion reach this same path. Existing
  vector backend restrictions/raster routing remain; the change does not grant
  native vector support for every colour space.
- Indexed image palette conversion now carries the requested colour-transform
  options into the base-space conversion instead of silently resetting policy.

`ColorReport.icc_alternate_usage` adds serde-defaulted counters for profile
conversion, explicit-policy alternate, unavailable-profile alternate, device
alternate and rejection. Its scope is explicitly cumulative on the current
thread, **not per document**; nested alternate calls count separately. A device
alternate counter identifies the selected family, not whether it was explicitly
declared or synthesized. Old JSON reports lacking the field deserialize with
default values.

## Source regressions and checks

Fifteen new regression functions cover implicit and explicit alternates,
paint/image consistency, no-paint alpha, clipping, Lab components, invalid
metadata, filtered-profile errors, nested/default-bound alternate graphs,
cancellation/depth cleanup, invalid samples/alpha/dimensions, the inline decoder,
Indexed backend-policy propagation and report compatibility. One feature-gated
case constructs an actual LittleCMS sRGB profile and asserts native output shape
and route counters. Dummy profile bytes in the other cases intentionally exercise
alternate routing; they do not represent successful ICC profile processing.

Two earlier regressions were adjusted: scope binding now materializes the
implicit ICC alternate without mutating the source, and an incomplete ICC array
is invalid rather than unhandled.

All regression functions are **unexecuted**. `rustfmt` parsing/formatting and
tracked whitespace inspection are the only automated checks performed. They do
not establish name/type/ownership correctness or executed rendering results.
No compiler, build, tests, PDF workloads, benchmarks, deployment, commit or push
was run for this increment.

## Standards basis

The ICCBased table specifies alternate component compatibility, implicit device
alternates, and clipping without rescaling at alternate handoff. Image Decode
defaults are a separate sample-domain rule:

- [ISO 32000-2 draft, ICCBased table 65](https://developer.adobe.com/document-services/docs/assets/5b15559b96303194340b99820d3a70fa/PDF_ISO_32000-2.pdf)
- [ISO 32000-1, image Decode table 90](https://developer.adobe.com/document-services/docs/assets/35e4369068f86065372c18787171a17e/PDF_ISO_32000-1.pdf)
- [PDF Association graphics errata, including required ICC versions](https://pdf-issues.pdfa.org/32000-2-2020/clause08.html)

## Still open

- The subsequent `image_sample_domain_implementation.md` adds row-aware Lab/ICC
  sample decoding, floating-point Decode values, source provenance for full
  scoped image paths and source-aware cache keys. Optimized source-alias routes,
  Indexed palette domains and higher-precision native CMM processing remain
  incomplete; the compatibility byte-converter still receives normalized bytes.
- Full supported ICC program/PCS/device-link coverage and precision remain
  bounded by the existing backends. Alternate fallback is not evidence of
  conformance with the specification's required ICC profile support.
- Profile/alternate preparation reuse, negative-profile caching and aggregate
  conversion budgets need further implementation and performance evidence.
- Thread-local counters are not transaction-scoped decisions or per-render
  proof. Paint's existing `NamedColor` interface retains a static rejection
  reason; image conversion propagates the richer engine error.
- Non-RGB blending, overprint/soft masks, native vector colour and the broader
  rendering/standards matrix remain open.
- All exact-revision compilation, regression and binding execution, independent
  extraction/render comparisons, real-PDF corpus and capacity qualification.
- The layout, accessibility, collaboration, font, scan and interactive-editor
  work tracked in `universal_editor_roadmap_tracking.md` remains in scope and
  incomplete. This report does not narrow the user's full-roadmap objective.
