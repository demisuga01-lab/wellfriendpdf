# Shading BBox, pattern backgrounds and exact cache parameters

Source continuation on `main` over
`27e62db3a1b84804339e65b6025273fd003b3736`, preserving the existing uncommitted
roadmap changes. No compiler, build, type check, tests, PDF/rendering workloads,
benchmarks, binding execution or deployment were run. The roadmap is not complete.

## Implemented source behavior

- Common shading entries now resolve indirect arrays and numeric elements,
  interpret optional null entries as absent, normalize opposite rectangle
  corners, and reject malformed/nonfinite BBox/background values. Background
  component counts come from the resolved colour-space graph. `/AntiAlias` is
  validated as an optional boolean; this change does not implement arbitrary
  function antialiasing.
- `/BBox` is transformed from the shading's target coordinate system, not the
  Type 1 function's `/Matrix` domain. Rotated, reflected and sheared rectangles
  retain their polygon rather than using only its axis-aligned envelope.
- The transformed BBox is clipped against the visible destination extent before
  scratch allocation. Empty/degenerate/disjoint bounds do not allocate a scratch
  surface. A small BBox no longer requires a full visible-page scratch buffer.
- Temporary BBox coverage uses constant-storage convex polygon clipping and
  local-origin area calculation. Axis-aligned rectangles use direct overlap
  area. Large finite polygons are clipped before per-pixel area calculation;
  nonfinite transformations/intersections return errors.
- BBox coverage reaches all seven native shading families and combines with the
  installed clip using the same minimum-coverage convention as `ClipMask`.
  The existing clip is not replaced or modified. Opacity, soft mask, blend mode
  and knockout state still pass through one final compositor, so temporary
  coverage is not multiplied into the result twice.
- Sampling observes zero-coverage holes in the original clip, as well as BBox
  exclusions. The scratch path no longer evaluates functions solely because a
  clipped-out pixel lies inside the clip's rectangular extent. Clip access is
  borrowed for sampling without cloning the full-page mask.
- Pattern invocation explicitly enables `/Background`; direct raw/retained `sh`
  does not. The background fills gaps in the shading domain/mesh inside the
  bounded paint region, using the same source colour graph and selected CMM
  policy. Background and foreground are composited together once. Both pattern
  fill and stroke paths carry this invocation distinction and their own alpha.
- BBox/background work passes participate in the shared work counter; scratch
  storage remains governed by the existing local/shared temporary-memory limits.
- Axial/radial cache buckets now require an exact parameter-bit match before
  reusing a colour. Different inputs sharing a bucket are recomputed. This
  prevents discontinuous functions from changing with sampling or clipping
  order. Cache storage reservations include the larger keyed entries.

## Specification basis

Adobe's shading dictionary defines the target-space temporary BBox clip and
limits background application to shading patterns, not direct `sh` paint.
[PDF Reference 1.6, shading dictionary](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf).

PDF rectangles may use either pair of diagonally opposite corners; consumers
normalize them when specific corners are needed.
[PDF Reference 1.3, section 3.8.3](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.3.pdf).

## Twenty new regression functions, unexecuted

Seven geometry/dictionary functions assert reference resolution, optional nulls,
normalization, fractional axis-aligned and sheared coverage, reflection,
degenerate/disjoint bounds, cropped/rotated coordinates, extreme finite bounds
and malformed entry handling.

Ten shading-entry/cache functions assert all-seven-family BBox/background
behavior, the Type 1 matrix distinction, fractional clip/opacity/soft-mask
composition, sheared coverage, original palette domains, skipped clip holes,
bounded scratch allocation, malformed-entry rejection and exact cache parameter
matching across axial/radial discontinuities and clipped sampling order.

Three renderer dispatch functions assert raw/retained direct behavior, fill and
stroke pattern backgrounds, and pattern-base rather than current-fill matrix
placement. These are source assertions, not successful rendered artifacts or a
complete retained-render/binding qualification run.

Parser-only and whitespace checks establish source hygiene only. Types,
ownership/lifetimes, compiled dispatch, assertions and pixels are not verified.

## Remaining implementation and qualification

- BBox polygon coverage uses floating-point arithmetic, not exact predicates or
  interval arithmetic. Minimum combination with an already rasterized clip is
  the current renderer's convention; it does not reconstruct an exact geometric
  intersection of two arbitrary fractional-coverage shapes.
- Shading/function sampling remains point-based. `/AntiAlias` filtering,
  arbitrary high-frequency colour functions, formal colour-error control and
  more precise edge sampling remain separate work.
- The exact cache key fixes an incorrect reuse path; no speed improvement is
  claimed. Prepared functions/transforms, native colour precision, prepress and
  SVG/PostScript parity remain unfinished.
- Shared/partial patch edges, localized refinement, folded surfaces, full
  transparency/overprint/non-RGB compositing and other codec/export boundaries
  remain in the renderer roadmap. The prior cancellation/resource limitations
  also remain: no total-RSS bound or rollback during final compositing is claimed.
- Exact-revision compilation, regressions, independent output comparisons,
  difficult PDF corpora, all bindings and performance/resource qualification
  remain unrun, alongside the wider font/layout/tag/history/scan/UI roadmap.

No universal editing, production readiness or superiority over Acrobat is proven.
