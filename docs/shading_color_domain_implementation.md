# Shading colour domains and mesh decoding continuation

Source implementation on `main` over
`27e62db3a1b84804339e65b6025273fd003b3736`, preserving the accumulated uncommitted
roadmap changes. No builds, type checks, tests, PDF/rendering workloads, binding
execution, benchmarks or deployment were run. This is not roadmap completion.

## Implemented

- Scope binding returns both original and remapped shading colour graphs without
  injecting private provenance entries into PDF dictionaries. Raw `sh`, retained
  shading descriptors, and fill/stroke shading patterns pass the original graph
  and current colour-transform policy into the shading renderer.
- Validation and actual conversion use the same supplied backend, intent and
  cache scope rather than silently selecting default colour-management options.
  Source-domain propagation reaches function-based, axial, radial, Gouraud and
  Coons/tensor patch colour evaluation, including Indexed palette interpretation
  and nested ICC alternates. Direct callers retain their existing entry points.
- ICC mesh component counts come from colour-space metadata, including `N = 1`
  and `N = 4`; unknown families no longer implicitly decode three components.
- Conversion errors and failed function evaluation at actual samples are
  retained as a per-call failure and returned by the cancellable entry point.
  Missing mesh streams, truncated Type 4 vertices/triangles, invalid edge reuse,
  incomplete lattice rows and malformed/truncated patch data no longer report
  successful partial shading. Patch-count exhaustion reports failure, with an
  exact-cap stream allowed to terminate normally.
- Type 4 flag-zero records start independent triangles and consume the next two
  records with their flags ignored. Edge reuse requires an established triangle.
- Lattice vertices advance to their next byte boundary. Complete row extent is
  checked against the available decoded bytes before reserving row storage;
  allocation uses `try_reserve_exact`. The dictionary alone cannot request a
  huge row allocation backed by a tiny stream. Mesh rendering borrows decoded
  bytes rather than cloning the complete stream again.
- Local cancellation is linked to the enclosing cancellation scope. Mesh vertex,
  row, patch and triangle-scanline processing polls it, and completion checks
  cancellation before reporting success. This remains cooperative cancellation,
  not forced interruption inside native colour-management calls.

## Added regression source

Twelve new unexecuted regression functions cover:

- Constant-colour source-domain agreement in all seven shading families.
- Selected ICC backend use during both validation and conversion.
- Gray/RGB/CMYK ICC mesh component counts and rendered coverage assertions.
- A calculator function that passes midpoint validation but fails at a real
  sampled position.
- Independent Type 4 triangle restarts, ignored continuation flags, rejected
  unanchored reuse, missing streams and truncated records.
- Byte-padded low-bit lattice vertices and oversized row metadata.
- Enclosing cancellation surviving a local uncancelled token.
- Raw and retained shading dispatch, and source-aware fill/stroke patterns.

Seven existing mesh-parser assertions now supply a reader for component metadata;
two float-colour-cache calls supply default shading options. The previous
resource-binding assertion was adapted to the explicit source/target result.

`rustfmt` parser-only checks and tracked/new-file whitespace inspection passed.
These checks do not resolve symbols, types, ownership or lifetimes and do not
execute any assertion. Constant-colour fixtures cannot establish correctness of
nonlinear interpolation or general colour fidelity.

## Standards basis

The vertex flag and byte-padding changes follow ISO 32000-1, sections
8.7.4.5.5–8.7.4.5.6, in the
[Adobe-hosted specification](https://raw.githubusercontent.com/adobe/dc-acrobat-sdk-docs/master/docs/standards/pdfstandards/pdf/PDF32000_2008.pdf).
Its mesh-function ordering also exposes a separate remaining algorithm defect:
interpolate source parameters before applying a nonlinear function, not already
converted vertex RGB colours.

## Remaining implementation, not just qualification

Follow-up: `shading_mesh_interpolation_implementation.md` supersedes the next
three original source-gap bullets with parameter/component interpolation,
single-composite opacity, mesh work charging and shared working-storage tokens.
It also records the still-open tessellation, precision, resource and runtime
qualification boundaries; the historical bullets below describe this report's
earlier snapshot, not the latest code.

- Mesh function evaluation and colour conversion still occur at vertices/corners
  before RGB interpolation. General nonlinear functions and nonlinear source
  colour spaces need source-component/parameter interpolation through both
  Gouraud rasterization and patch subdivision. That is the next core mesh task.
- The shading path still needs correct graphics-state opacity propagation and
  shared-edge coverage/compositing treatment. Merely adding alpha to duplicated
  boundary samples would introduce opacity seams.
- The cumulative shading pixel budget currently covers function/axial/radial
  work, not every mesh triangle/subdivision. Mesh raster work and decoded-vertex
  memory need complete aggregate budgeting; stream-length checks and a patch
  count cap are not a measured memory/latency guarantee.
- Native vector exports, prepress reporting, codec/component-export provenance,
  high-precision ICC, overprint/soft-mask/non-RGB blending and full colour-space
  conformance remain separate work. Direct low-level legacy void APIs log
  failures; the cancellable result-returning route propagates them.
- Current compilation, bindings, independent visual/extraction oracles, difficult
  PDF corpora, cancellation and resource/performance qualification remain unrun.
- The complete editing/layout/tag/font/history/scan/UI roadmap remains active.
  No universal, production-ready or better-than-Acrobat claim is made.
