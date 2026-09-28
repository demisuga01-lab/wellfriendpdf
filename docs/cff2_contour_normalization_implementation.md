# CFF2 contour normalization — source implementation

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This continues the
full editor/rendering roadmap. It adds an actual numerical contour-union route
to CFF2 publication, not just an overlap warning. It is not compiled, executed
or visually qualified. The broader roadmap remains incomplete.

## Explicit policy, not silent hint loss

`FontInstanceRequest` now accepts optional `cff2_contours`:

```json
{
  "tolerance_font_units": 0.001,
  "allow_hint_loss": false
}
```

The object is supplied inside the existing source-bound font-instance request.
Its tolerance is in font design units, not page points or device pixels. Omitting
the object preserves the source contour/hint program only after a compatibility
check and rejects outlines needing reconstruction. Supplying it requests
nonzero-winding normalization. It is rejected for a non-CFF2 source; this field
is never silently ignored.

Normalization preserves compatible glyphs' expanded static programs and frozen
private dictionaries exactly. It only requires `allow_hint_loss: true` when a
glyph actually needs contour rewriting and has stem/mask/flex hints or private
hint parameters. The rewritten glyph then uses an unhinted private owner, while
compatible glyphs sharing its original owner retain their hint program. Hint-only
empty glyphs are retained without loss consent. Consent permits necessary loss;
it does not request blanket dehinting. Recomputed auto-hinting and exact
source-hint reassignment on genuinely rewritten contours are not implemented.

The local font picker exposes separate normalization, tolerance and hint-loss
choices for CFF2. It defaults to preserving source contours; consent is not
preselected. Preparation does not itself modify a PDF or approve a checkpoint.
Native and browser capability gates use `font_instance_protocol_version: 4`.
Existing native/managed JSON transports carry the same command. The worker
snapshots the nested decision before queuing it.
The selective-preservation increment does not change the request schema. Its
additional report fields default when reading older receipts and are optional
in browser typings for older replies; their absence is not evidence
that an earlier implementation retained hints.

## Curve-preserving numerical route

`fonts/cff2_contours.rs` consumes only the SDK's already-expanded CFF1 operator
subset. It is not another arbitrary-font/subroutine parser. It resolves paths,
opaque mask bytes and all four flex forms using f64 coordinates. Compatible flex
operators retain their depth/operands. Rewritten flex geometry becomes ordinary
cubic curves only under the explicit normalization and hint-loss decisions.

The implementation uses pinned `linesweeper 0.4.0` and `kurbo 0.13.1`. The former
describes itself as early beta and implements a sweep-line topology model;
its `contours_correct` route reconstructs curve boundaries with stronger ordering
checks than the ordinary contour route. This is the selected integration, not
an assertion that the dependency or this SDK is production-qualified. See the
[upstream source](https://github.com/jneem/linesweeper) and
[public API](https://docs.rs/linesweeper/0.4.0/linesweeper/).

For each glyph, the route:

1. Expands its exact selected source program. Straight-edged outlines first use
   exact bounded-integer contacts and winding checks, retaining compatible
   programs without calling the numerical solver. See
   `cff2_exact_linear_implementation.md`. For curved outlines a numerical topology check includes
   all source-edge records (including merged/deleted records), requires unit ink
   winding adjacent to zero outside winding, and requires degree-two vertices.
   Compatible boundaries in either orientation retain their program bytes;
   hint-only empty glyphs also retain their bytes. Crossing/touching vertices,
   redundant ink and cancelled contours do not bypass rewrite approval.
2. For remaining glyphs, enforces the hint-loss decision and resolves the
   nonzero-filled region, including intersections, coincident edges,
   holes and winding cancellation, then reconstructs curve boundaries.
3. Emits cubic/line Type 2 operations with absolute points quantized to 16.16
   before taking relative deltas. This avoids cumulative coordinate rounding.
4. Decodes those actual emitted bytes, builds their topology and rejects a
   non-normal boundary. Closing a contour retains the Type 2 current point.
5. Computes a numerical symmetric-difference area between input and saved filled
   regions and rejects differences above the declared area guard.

The solver epsilon is tolerance/8. The area guard is
`4 * tolerance * (source_perimeter + 1)`. This is an engineering guard, not a
certified Hausdorff-distance or pixel bound. The recheck and comparison use the
same topology library and are **not independent validation**. Features near the
numerical resolution can collapse or change classification. The report discloses
emptied glyphs as well as normalized, dehinted, preserved and preserved-hint
glyph IDs. Curved compatibility uses the same numerical model, not an
exact-arithmetic geometric certificate. Preserved bytes do not incur additional
contour quantization. The separate `exact_linear_preserved_glyphs` receipt names
straight-edge decisions made without numerical tolerance; it does not claim
independent rendering qualification.

Private dictionaries are interned as shared immutable byte buffers, bounded to
32 MiB of unique frozen bytes. Output FDs are assigned after glyph decisions.
Only retained dictionaries count toward the CFF1 256-FD limit: more than 256
distinct retained semantics are still rejected, not silently merged or dehinted.
The report distinguishes dictionaries bypassed for rewritten glyphs, retained on
unchanged glyphs, and removed entirely; a shared owner may be both bypassed and
retained. Exactness here is relative to the selected, expanded static program and
frozen private values, not identity with a variable CFF2 font's source bytes.

The need to normalize nonzero-filled regions, rather than blindly switch fill
rules, follows the format distinction in the
[OpenType glyph-format comparison](https://learn.microsoft.com/en-us/typography/opentype/spec/glyphformatcomparison).
No external converter or font executable is invoked.

## Metrics and atomic publication

Normalized outline bounds can change even when the filled region does not:
cancelled contours and redundant cubic control hulls need not survive union.
The complete-font transaction therefore rebases geometry before publication.
It retains shared original metric-table buffers, refreezes HVAR/VVAR/MVAR and
horizontal/vertical metrics against the emitted glyphs, and confirms that
selected horizontal advances have not changed. It does not apply variation
deltas to previously varied table bytes. New redundant-metric discrepancies
still require the existing explicit acceptance flag.

The emitted CFF FontBBox, head bounds and metric headers agree with the emitted
outline domain. A head sidebearing-equality flag is cleared when contradicted by
the selected metrics. Names' unique identity now includes the normalization
decision. Existing exact table receipts, CFF/sfnt readback, permissions and
signature decisions still gate final asset publication. A failure returns no
partially registered font or changed session PDF.

## Resource and cancellation boundaries

Tolerance must be finite and in `1/65536..=0.125` design units. Source control
coordinates must be finite and within +/-1,048,576 units. Per-glyph limits are
2048 source segments, 4096 output path elements and 65535 emitted bytes; aggregate
input/output segment counts are each capped at one million.

Control-hull broadphase checks run before source, saved-output and comparison
solves. Degree products bound potential exact curve intersections among candidate
pairs. Each solve is limited to 250,000 such candidates, and the transaction to
16 million broadphase pair checks. These are preflight complexity guards, **not
a hard cap on all allocations or iterations inside the dependency**. Existing
font/table/output bounds also apply.

Cancellation is checked in SDK loops and around each synchronous solver call.
The upstream solver itself is not cooperatively interruptible; hard deadlines
and measured worst-case memory remain qualification/integration work. Worker
termination remains the existing browser hard-cancellation mechanism.

## Dependency provenance and regression source

The registry archive for linesweeper 0.4.0 was read without executing it; its
SHA-256 matches
`9c19728333c060c6569a53c9a0e56c4be0df52cb4e6e07a8fbe16084cecce769`.
Its normal dependencies already have compatible versions in the workspace
lockfile. The new package/direct references and serde-enabled dependency edges
were entered manually; Cargo resolution has **not** been run. The MIT notices
are included under `crates/engine/licenses` and in package inclusion rules.
The old aggregate licence/dependency audit is not claimed as current.

Sixteen new regression functions remain unexecuted: fourteen geometry/public-font
cases, one retained-session case and one worker-client snapshot case. Cases
cover rectangle and cubic unions, holes/islands, coincident/opposite contours,
self-crossings, closure/flex semantics, budgets/cancellation, hint consent,
empty glyphs, emitted metrics, avoiding duplicate MVAR application, asset
identity and repeated saved-story editing. The existing capability case advanced
to version 3 for normalization; checked default publication now advances the
shared font protocol to version 4.

The subsequent selective-preservation increment adds eight more unexecuted
regression functions (seven font/topology/report cases and one full retained-story
flow), and updates the existing empty-hint case. These check byte preservation in
both orientations, mixed shared-private selection, no-consent rejection for
merged/cancelled/touching contours, flex depth preservation, aggregate budget
enforcement, post-decision FD capacity, older report decoding and save/reopen/edit
with a hinted font prepared without loss consent. These are regression source,
not executed results. Formatting/parser and scoped Git whitespace checks passed.

Only source review, formatting/parser and whitespace checks are authorized here.
No Cargo/compiler/build/typecheck, test, font/PDF workload, renderer, benchmark,
browser QA, commit, push or deployment was run. Real-font corpus behavior,
independent geometry/raster comparisons, dependency-resolution/build results,
native/binding/browser behavior and adversarial solver performance remain
unverified. Exact hint-preserving union, automatic rehinting, wider font owners
and the full editing/rendering roadmap remain open.
