# Image sample-domain continuation

Source-only work on `main` over `27e62db3a1b84804339e65b6025273fd003b3736`, extending
the uncommitted roadmap candidate. No build, compiler, tests, PDF workloads,
rendering, benchmarks, deployment, commit or push was performed.

## Implemented

`images/sample_decode.rs` reads packed 1-, 2-, 4-, 8- and 16-bit components with
checked row sizes, exact input length, row padding and big-endian 16-bit handling.
It applies PDF Decode mappings in floating-point rather than discarding low-order
bits or clamping decoded components to the unit interval first. Descending,
constant, signed and non-unit mappings are supported; malformed types, counts,
unresolved references and non-finite endpoints are rejected. Decode and endpoint
references resolve through the reader when available. An overflow-resistant
interpolation expression handles opposing finite endpoint extrema.

The canonical raw-image builder now routes Lab and ICCBased images through this
path before bit-depth normalization. Lab conversion applies explicit Decode or
the Lab defaults, clips to the actual component ranges and then converts to RGB.
ICC conversion receives lazily decoded component vectors without allocating a
whole floating-point image. Its alternate path retains those values through
range clipping and colour conversion; its bounded memoization keys retain the
floating-point component values rather than prematurely quantized bytes.

The existing ICC profile backend still accepts eight-bit input. This increment
does **not** establish sixteen-bit native ICC processing, even though source
unpacking and alternate conversion now retain the extra precision. Final output
remains the existing eight-bit `RawImage` representation.

Source-space provenance now travels separately from the effective colour-space
dictionary for the ordinary XObject decode and terminal DCT handoff, XObject raw
windows/scaled DCT where their original dictionaries identify the source, and
the full inline-image decoder. A source ICC space uses its original `/Range` as
the default Decode; device-to-ICC remapping does not replace that device default
with the replacement profile's range.

For canonical full-page rendering, full-image XObject and full-inline paths now
resolve source resource aliases without applying device defaults. The effective
render graph remains separately default-bound. Both source and effective graphs
participate in those image cache identities. Two scopes with identical effective
profiles but different original Decode domains therefore do not intentionally
share a decoded-image entry. Regional SVG/PostScript callers pass their existing
resolved source spaces to the extended internal decoder signature.

Cancellation is checked during conversion and before returning output; output
allocation remains bounded. The original source dictionaries are not mutated.

## Regression source and static checks

Fourteen new sample/domain regressions cover packed row boundaries, low-order
16-bit data, invalid sizes and component access, malformed/descending/constant
Decode, finite extreme endpoints, Lab clipping, signed ICC-to-Lab alternates,
original-versus-remapped ICC defaults, full-inline integration and cancellation.
One additional renderer regression deliberately uses the same image and identical
replacement graph in two resource scopes, asserting different source-bound cache
keys and different full-image decode output.

These fifteen regressions are **unexecuted source**. The renderer regression
calls the real scheduled decode path but is not observed pixel evidence. Parser
checks with `rustfmt` and whitespace inspection do not establish compilation,
ownership/type correctness, execution, fidelity or interoperability.

## Specification basis

PDF image Decode maps sample integers into component domains before colour
conversion. Lab and ICC defaults are not generally unit ranges; Indexed defaults
have a distinct index rule. See the primary
[ISO 32000-2 draft, default Decode table 88 and section 8.9.5](https://developer.adobe.com/document-services/docs/assets/5b15559b96303194340b99820d3a70fa/PDF_ISO_32000-2.pdf)
and [ISO 32000-1 table 90](https://developer.adobe.com/document-services/docs/assets/35e4369068f86065372c18787171a17e/PDF_ISO_32000-1.pdf).

## Work still required

- The subsequent `indexed_and_optimized_image_domains.md` increment carries
  resolved source domains through raw-window/scaled-JPEG paths and routes Indexed
  images through shared pixel Decode and source-base palette conversion. Its
  regressions remain unexecuted. Non-image Indexed paint, other codec integration
  and qualification remain open; that later report records the exact boundary.
- Component-selected raw exports, JPX colour interpretation and general
  higher-precision profile transforms/output require additional integration.
- Native CMM eight-bit quantization, arbitrary ICC programs and other renderer
  colour/compositing boundaries remain as documented in the ICC report.
- Cache preparation/graph resolution overhead needs profiling; no speed claim is
  made for repeatedly resolving source colour spaces for cache identity.
- All exact-revision build, regression, binding, independent-render, real-PDF
  corpus and performance qualification remains pending under the user's ban on
  running them locally.
- The full editing, layout, tags, fonts, collaboration, scan and UI roadmap is
  still active and incomplete. This is an implementation increment, not universal
  editing or evidence of superiority over Acrobat.
