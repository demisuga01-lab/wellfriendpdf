# Compatible, device-adaptive patch tessellation

Source continuation on `main` over
`27e62db3a1b84804339e65b6025273fd003b3736`, preserving the accumulated worktree.
No builds, compiler/type checks, tests, PDF/rendering workloads, benchmarks,
bindings or deployments were run. The full roadmap remains active.

## Implemented source path

The Type 6/7 renderer now collects bounded patch records before painting them.
Coons boundaries are degree-elevated into a bicubic control net; Type 7 keeps all
sixteen controls. Both become device-space nets under the current transform.
This replaces the per-patch, user-space subdivision heuristic.

The planner estimates second-derivative bounds from Bernstein control
differences. With `Muu`, `Mvv` and `Muv` bounding vector derivative norms, the
uniform grid's exact-arithmetic geometric error estimate is:

`E(n) <= (Muu + Mvv)/(8 n^2) + Muv/(4 n^2)`.

The first term bounds interpolation to a bilinear cell; the second bounds
replacing that cell by two triangles. The initial factor is the smallest power
of two meeting a quarter-device-pixel estimate, up to 1024 steps per axis.
Nonfinite estimates or refinement beyond that cap return an error, not success
with silently degraded geometry. This is an engineering error estimate using
floating-point arithmetic, not an interval-certified bound. `/SM` remains a
colour smoothness setting; it is no longer used to reduce geometric accuracy.

Each patch has two axis factors. A canonical key matches complete cubic
boundary controls, including reversed order and normalized signed zero. A
bounded sorted edge index and disjoint-set union propagate the maximum required
factor across corresponding axes. This covers flag 1/2/3 reuse, independent
patch records with identical boundaries, and transitive connections; unrelated
axes and disconnected patches need not share the maximum factor.

Boundary evaluation uses the canonical control order and integer-index reversal
on dyadic parameters. The same edge receives identical device positions from
both patches. Endpoints, collapsed boundaries and palindromic control polygons
have explicit consistent evaluation. Interior points use de Casteljau evaluation.
The earlier half-open triangle rasterizer then assigns identical shared segments
to one triangle. Source corner colour/function interpolation remains unchanged.

The renderer retains source patch order, including overlapping patches, and
uses two reusable vertex rows instead of an entire grid. Work charges cover
planning, collection growth, bounded topology sorting and vertex/raster work.
Byte reservations cover collection capacity, simultaneous old/new arrays during
growth, edge/axis storage, and the two rows. Shared-memory tokens are released
on error/success; allocation uses fallible reservation. Completely off-screen
control hulls, with a one-pixel margin, do not consume visible refinement work.
Parsing, topology and row/raster processing observe cooperative cancellation.

## Mathematical and research basis

The derivative construction follows the control-difference description in
[Michigan Tech's Bezier derivative notes](https://pages.mtu.edu/~shene/COURSES/cs3621/NOTES/spline/Bezier/bezier-der.html).
The combined interpolation estimate above is the implementation's derivation
from those derivative bounds, not a theorem quoted from that source.

Matching boundary factors and consistent evaluation are also central to
[AMD's discussion of crack-free Bezier subdivision](https://gpuopen.com/learn/gpu-view-adaptive-subdivision/).
This implementation uses CPU axis negotiation and streaming rows; it does not
implement or claim the paper's GPU work graphs, recursive wedge triangulation,
performance results or complete guarantees.

The Coons/tensor source layout and reuse indices follow the existing SDK mapping
of ISO 32000-1 sections 8.7.4.5.7-8.7.4.5.8. The independent Coons formula and
Bernstein tensor evaluator are retained under `cfg(test)` as regression oracles.

## Seventeen new unexecuted regression functions

Fifteen planner/raster source regressions assert:

- Coons degree elevation and tensor control/surface agreement.
- Zoom-dependent factors, translation stability and mixed-derivative refinement.
- Different-curvature neighbours negotiating shared factors without globally
  refining disconnected patches.
- Bitwise boundary agreement for all three reuse flags, changed axes, reverse
  direction, signed zero, symmetric and collapsed controls.
- Transitive constraints in both record orders.
- Dense geometric probes against the derived error estimate.
- Nonfinite/refinement-cap errors and off-screen hull handling.
- Separate collection/topology/row/growth budgets, token release and cancellation.
- Two-row working storage fitting its calculated limit.

Two full shading-entry regression functions assert page coverage/opacity across
curved reused edges for both patch families and preservation of overlapping
patch paint order. The obsolete heuristic-count test was removed; its count
function is no longer the renderer's algorithm.

Parser-only and whitespace checks do not resolve types, lifetimes, ownership,
bindings or execute these assertions. No observed rendering or pass rate is
claimed.

## Still open

Follow-up: `shading_bounds_background_implementation.md` adds target-space BBox
clipping, pattern-only backgrounds, original clip-hole sampling and exact
gradient-cache parameter matching. It supersedes the BBox/background omission
below, but not the remaining function filtering, geometry, export or execution
qualification limits.

- The topology matcher identifies complete identical cubic control polygons,
  not every mathematically coincident curve, differently parameterized edge,
  or edge represented by several subcurves. General partial-edge stitching is
  not established by these source changes.
- Factors are uniform along each patch axis. More localized subdivision,
  geometric antialiasing, arbitrary folded/singular parameterizations and formal
  floating-point error bounds remain separate work.
- Geometric tolerance does not bound colour error or aliasing from arbitrary
  nonlinear functions. Prepared function/CMM evaluation and native precision,
  transparency, BBox/background and export conformance remain open.
- Budgets bound the explicit owned storage described here, not every allocator,
  native/profile/cache allocation or process RSS. Sorting is bounded and polled
  before/after, not interrupted inside each comparison. Cancellation during
  final destination compositing still returns failure without buffer rollback.
- Exact-revision compilation, regressions, bindings, independent pixel/extraction
  checks, difficult PDF corpora, resource/performance qualification, and the
  wider editing/font/layout/tag/history/scan/UI roadmap remain incomplete.

This is not universal, production-ready or better-than-Acrobat evidence.
