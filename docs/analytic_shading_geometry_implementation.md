# Analytic shading numeric geometry - source implementation

Continuation on `main` over `27e62db3a1b84804339e65b6025273fd003b3736`.
Existing roadmap changes remain uncommitted. This is not a full-roadmap
completion report or an executed rendering qualification.

## Source changes

`render/shading_geometry.rs` now supplies the Type 1/2/3 shading mapper with:

- Independently row-normalized affine inversion without an absolute determinant
  cutoff. Tiny/large uniform units, reflection and anisotropic row scales no
  longer become singular merely because their determinant has small magnitude.
  Singular or nonrepresentable inverse coefficients remain errors. This is
  local to analytic shading, not a replacement of every transform implementation.
- Axial projection with separate axis/displacement normalization. A short axis
  is not rejected by the previous `length_squared < 1e-10` cutoff; subtraction
  has a half-scale fallback when opposite finite endpoints overflow. A distant
  perpendicular sample does not set the axis normalization scale. Extensions
  still require the relevant `/Extend` decision.
- Radial source-coordinate normalization, compensated coefficient summation and
  an FMA-corrected difference of products for the discriminant. Vieta's relation
  calculates the cancellation-prone root. Linear, double-root and common-generator
  cases are explicit; a negative radius does not become a valid circle merely
  because the equation was squared. The largest valid parameter wins.
- Direct concentric-circle evaluation using `hypot`, avoiding a second spurious
  root and a reconstructed negative zero-radius boundary.
- Squared normalized coordinates and discriminant products have explicit
  underflow checks. Extreme dynamic range returns a numeric-range error instead
  of silently treating a lost quadratic coefficient as an exact linear case.
- Negative source radii are rejected by both dictionary validation and the
  geometry constructor. Two zero radii leave the shading unpainted.
- Endpoint-exact, overflow-aware function-domain interpolation, also used by
  the renderer's shading-function validation sample. Opposite finite domain
  endpoints no longer require evaluating an overflowing `end - start`.
- Nonfinite sample mappings and unrepresentable inverses propagate through the
  existing error channel before the scratch surface is composited. This does
  not add rollback to cancellation during final compositing.

The existing original colour-domain handling, exact-parameter cache, pattern
background, BBox coverage, opacity, work limits and cooperative cancellation
remain on the same paint path. No new public SDK or binding operation is needed.

## Algorithm and specification references

The numerical design uses compensated products and a cancellation-avoiding
quadratic root formulation described in
[Physically Based Rendering, Mathematical Infrastructure](https://pbr-book.org/4ed/Utilities/Mathematical_Infrastructure).
This implementation does not inherit that publication's error bounds for the
whole renderer or claim interval/exact arithmetic.

PDF radial shading requires nonnegative radii, defines no paint when both radii
are zero, and chooses the later valid blend circle with endpoint colours for
approved extensions.
[Adobe PDF Reference 1.4, section 4.6, Table 4.28](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf).

## Fourteen new regression functions, not executed

Ten geometry-level functions cover uniform-unit changes, translated/rotated
axes, overflowing finite endpoint differences, distant perpendicular samples,
growing/shrinking circles, both extensions, root ordering and radius validity,
linear/tangent/common-generator cases, cancellation-prone roots, finite extreme
function domains, affine inverses and invalid inputs.

Four paint-entry functions exercise Type 1/2/3 pixels across unit changes,
extreme-domain function validation and paint, radius validation/zero-radius
behavior, and failure without compositing the private scratch output.

Only rustfmt parsing/formatting and whitespace inspection were performed.
No compiler, type check, build, regression execution, PDF workload, pixel
rendering, benchmark, binding execution or deployment ran. These test functions
are specifications in source, not observed successful pixels.

## Still open

Finite f64 arithmetic is not an exact geometric predicate. Ill-conditioned
configurations, extreme ratios that exceed representable intermediates,
subnormal precision beyond those explicit guards and cancellation already present
in supplied or transformed coordinates are not generally certified. The
shading inverse can still refuse a finite matrix when its intermediate or
inverse translation cannot be represented. Broader source parameter/reference
normalization, filtering, function preparation, full colour/transparency,
codecs, native export and other renderer paths remain in the roadmap.

The larger editor/font/layout/tag/history/scan/browser roadmap and all current
executable qualification remain open. No universal or Acrobat-superiority
claim follows from this increment.
