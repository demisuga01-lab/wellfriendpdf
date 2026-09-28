# Mesh interpolation, shading opacity and working storage

Source continuation on `main` over
`27e62db3a1b84804339e65b6025273fd003b3736`; accumulated uncommitted changes are
preserved. This is not completion of the full editing/rendering roadmap.
No compiler, build, type check, test, PDF workload, rendering, benchmark,
binding execution or deployment was run.

## Source changes

- Gouraud vertices retain decoded source components or their single function
  parameter. Barycentric interpolation happens before function evaluation and
  colour-space conversion. This removes the previous endpoint-RGB interpolation
  for nonlinear functions, calibrated colours, ICC and tint transforms.
- Coons/tensor vertices retain surface `(u, v)`. The raster sample interpolates
  those coordinates, evaluates the four source corners bilinearly, then applies
  the function and colour conversion. Geometry remains a bounded triangulated
  approximation of the surface; the change does not establish exact geometry.
- Triangle edges use a half-open coverage rule with canonical endpoint order.
  Oppositely directed, identical edges have complementary ownership. Zero-area
  triangles do not paint; nonfinite geometry/components and invalid sample or
  patch arities become errors rather than partial successful output.
- The result-returning shading entry point paints into a transparent, clipped-
  extent scratch surface. Only after successful conversion/work checks does it
  composite to the destination. This applies the destination clip, soft mask,
  blend state and graphics-state opacity once, including overlapping triangles.
  Raw/retained `sh` and fill shading patterns use nonstroking alpha; stroke
  shading patterns use stroking alpha. The original destination clip is retained.
- Scratch geometry and ordered-dither phase retain full-page device origins,
  including cropped, rotated and tiled viewports. Scratch allocation checks
  arithmetic, uses fallible reservation, and has an explicit byte limit.
- The shared work counter now charges scratch clearing/compositing, decoded
  vertices, patch subdivision, and each triangle's clipped raster bounding box.
  A budget failure is returned before scratch pixels reach the destination.
- Local byte limits include scratch, a fixed parser/vertex allowance, axial/
  radial lookup storage, and simultaneous lattice-row or patch-grid storage.
  Public rendering supplies `RenderResourceBudget.max_temporary_bytes` and the
  existing shared temporary-memory budget. Shading takes nonblocking memory
  tokens: nested rendering cannot wait indefinitely on a surface held by its
  own ancestor on the same thread. Tokens release on success and early return.
- Malformed lattice rows and nonfinite samples record failures even after
  earlier rows were rasterized. Patch assembly rejects excess as well as
  insufficient inputs and invalid edge flags.

The source interpolation order follows ISO 32000-1 sections 8.7.4.5.5 through
8.7.4.5.8 in the
[Adobe specification repository](https://raw.githubusercontent.com/adobe/dc-acrobat-sdk-docs/master/docs/standards/pdfstandards/pdf/PDF32000_2008.pdf).

## Regression source, not execution evidence

Seventeen new regression functions add assertions for:

- Type 4/5 quadratic-function interpolation and nonlinear calibrated colour.
- Type 6/7 bilinear source corners before a quadratic function.
- Identical shared diagonal ownership in both windings, with half-coverage
  alpha exposing repeated samples.
- All seven families applying opacity, fractional clip and soft mask once;
  overlapping independent triangles not accumulating graphics-state opacity.
- A late function error and late cumulative mesh-work exhaustion preserving
  destination bytes.
- Scratch, row, grid and shared-memory limits; nonblocking nested rejection;
  reservation release after error/success; checked allocation overflow.
- Cropped/full/tiled dither agreement under all four right-angle rotations.
- Invalid opacity, sample arity, nonfinite components and malformed patches.
- Raw/retained opacity dispatch, distinct fill/stroke pattern alpha, and the
  public resource contract reaching scratch allocation.

Parser-only `rustfmt` and whitespace checks are source hygiene, not symbol,
type, lifetime, ownership, pixel, interoperability or performance verification.
Assertions are future qualification material, not observed passing results.

## Remaining implementation and qualification

The subsequent `shading_patch_tessellation_implementation.md` replaces the
per-patch subdivision and curved shared-edge limitation in the next historical
bullet for complete matching cubic boundaries. It adds device-space estimates,
axis negotiation and streaming rows. Its own report retains partial-edge,
aliasing, folded-surface and executable qualification boundaries.

- Curved shared patch boundaries still need edge-compatible tessellation.
  Tensor subdivision depends on individual patch curvature, so neighbouring
  patches can use different boundary sample counts. Complementary ownership of
  identical straight edges does not resolve those T-junctions. Device-space
  adaptive error bounds, degenerate/folded surfaces and geometric antialiasing
  also remain separate work.
- Source-space colour evaluation currently performs function/CMM conversion per
  raster sample. The work budget bounds iteration, not native-call cost. Prepared
  function/colour transforms and performance qualification remain necessary.
- The memory bound covers the owned buffers described above, not all function,
  colour-profile/cache/native allocations, allocator overhead or total RSS.
  Nested reservations fail explicitly rather than growing the budget or waiting.
- Cancellation is cooperative. Conversion/work failures leave the destination
  unchanged; cancellation during final compositing may leave its private buffer
  partially changed while returning an error. Callers must not publish a failed
  page result. No rollback or native-call interruption guarantee is made.
- Legacy void rendering helpers still log errors instead of returning a typed
  result. Broader BBox/background, transparency/overprint, precision, codec and
  native-vector-export conformance remain part of the open renderer roadmap.
- Current compilation, all bindings, independent rendering/extraction oracles,
  difficult real PDFs, cancellation and memory/latency evidence remain unrun.
  Editing, layout, tags, fonts, collaboration, reconstruction and production UI
  work in the main roadmap is not closed by this renderer increment.

No universal-editing, production-readiness or better-than-Acrobat claim follows
from these source changes.
