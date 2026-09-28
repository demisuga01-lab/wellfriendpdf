# Checked default CFF2 publication - source implementation

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736` in the existing dirty
roadmap worktree. This closes one silent-compatibility path; it does not complete
the editor/rendering roadmap or qualify universal PDF editing. No compiler,
tests, font/PDF workloads, rendering, benchmarks, browser QA or deployment ran.

## Default contract

A CFF2 source cannot be copied directly into a CFF1 table: the formats differ,
and CFF2 permits overlapping nonzero-winding contours while legacy CFF consumers
do not have the same requirement. The SDK already expanded CFF2 variation,
subroutines, widths and selected private semantics before emitting CFF1. The old
default then retained expanded contour programs without checking compatibility.

The default `cff2_contours: null` transaction now:

1. Expands the selected glyph and private programs exactly as before.
2. Classifies empty/zero-ink and straight-edge outlines with the bounded exact
   16.16/i128 route documented in `cff2_exact_linear_implementation.md`.
3. Classifies curved outlines with the bounded topology route at the minimum
   16.16 tolerance and records that this is numerical, not independent proof.
4. Preserves glyph/private bytes and all selected hints only if every glyph has
   a compatible unit/zero nonzero-winding boundary.
5. Refuses atomically at the first glyph needing reconstruction. The typed error
   directs the caller to request `cff2_contours` normalization and, only when the
   rewrite owns hints, separately approve hint loss. The default never silently
   reconstructs, dehints or publishes unchecked contours.

Flat line and flex programs whose endpoints and all controls are collinear have
zero winding and are retained. This avoids treating harmless zero-ink source as
a filled overlap. Noncollinear curves still use the numerical route; no curve is
flattened for the default decision.

## Receipt and protocol

Successful CFF2 publication now always sets `contour_overlaps_checked: true`.
When bytes were merely preserved, `contour_overlaps_removed` remains false and
the additive `preserved_contour_check` report records exact-linear IDs, preserved
hint IDs, solver/work counters and `independently_verified: false`. Opted-in
normalization continues to use `contour_normalization` and sets the removed flag
after serialized-path checks. Older serialized CFF reports lacking the new
optional receipt deserialize as `None`; absence never fabricates evidence.

The retained-session status and browser gate advance
`font_instance_protocol_version` from 3 to 4. A current picker therefore cannot
send a default-preservation request to an older worker that still accepts
unchecked outlines. The request JSON itself remains compatible: omission still
means preserve, but version 4 gives preserve a stricter fail-closed contract.
Browser copy explains the checked default and explicit reconstruction route.

## Regression source and remaining evidence

Four new Rust regression functions remain unexecuted, with existing tests also
extended. They cover default rejection for hinted/unhinted overlap, compatible
exact-linear publication, curved compatibility, flat-flex retention, old-report
decoding, native-session atomicity and read-only preparation. Browser capability
regression source now expects version 4. Formatting/parser and scoped whitespace
checks are the only permitted local validation.

Compilation, test execution, binding/browser execution, real fonts, independent
raster comparison, interoperability and adversarial resource evidence remain
pending. Curved classification and reconstruction still use the same numerical
topology family; rewritten-outline hint transfer/rehinting remains absent. The
larger typography, layout, accessibility, object, collaboration, rendering,
security and qualification roadmap remains active.
