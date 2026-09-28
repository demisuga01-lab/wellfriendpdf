# Tuple variations and CVT staging - source increment

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This is further
implementation toward complete font preparation, not completion of that feature,
the editor/rendering roadmap, or executable qualification.

## Implemented source

`fonts/tuple_variations.rs` decodes the tuple stores shared by cvar and gvar:
embedded/shared peaks, intermediate regions, shared/private point lists, and
signed-byte, signed-word and zero delta runs. Payload sizes bound each tuple;
neither a run nor a truncated payload can consume the next tuple. Inactive
tuples are still structurally decoded and checked. Duplicate point references
remain distinct until their deltas accumulate. Point numbers use cumulative
checked indices, including phantom indices above 65,535; a zero point-count
marker means the complete target domain, not an empty selection.

Simple-glyph inference uses original explicit point coordinates, contour
boundaries, and a touched-point bitmap for each tuple. An explicitly specified
zero is not confused with an absent value. Interpolation includes wrap-around,
equal-coordinate handling and single/no-reference contours, and never crosses
contour boundaries. Components, phantom points and CVT entries do not receive
this contour interpolation. Scaled contributions accumulate in f64 without
per-tuple integer rounding. Scalar rules are shared with ItemVariationStore.

`fonts/gvar_instance.rs` retains an immutable table and owned instance
coordinates. It validates the source glyph/axis counts, short or long offset
arrays, ordered bounded glyph extents, shared-peak records and disjoint owners.
It resolves individual glyph tuple stores through the shared decoder, and returns
explicit zero deltas for a genuinely empty glyph variation entry. A caller can
carry one evaluation budget across glyphs. Directory preparation has its own
reported work count. No mutable source or alternate coordinate array can be
substituted between calls.

`fonts/cvar_instance.rs` resolves CVT deltas, then rounds the final base-plus-delta
value once with checked signed-FWORD bounds. The resulting CVT retains every
original index and entry order, and reports changed values, tuple counts and work.
It does not execute, rewrite or remove TrueType instruction programs.

`font_metric_instance::prepare` stages cvar with the same source face and
normalized coordinates used by the existing metric/layout transaction. A missing
CVT owner or a cvar table on CFF outlines is rejected. Non-variable sources ignore
stray cvar data. Variable TrueType sources also retain the checked gvar directory
for the next point-preserving outline-writing stage. Failure returns no combined
stage; this entry still does not publish a partially instanced font.

## Resource and evidence boundaries

Tuple and gvar source tables are limited to 64 MiB, target domains to one million
entries, normalized axes to 64, and each tuple evaluation budget to 16 million
work units. Domain lengths and work are checked before large output allocations.
Shared point arrays use Arc ownership. Contour inference walks each contour
linearly instead of searching for neighboring references independently for every
point. Parsing, contour work, accumulation and publication poll cancellation.
These are component limits, not measured whole-process memory or latency bounds.

Forty-one new regression functions remain unexecuted: 22 tuple-decoder/inference,
10 directory/evaluation, and nine CVT/source-composition regressions. Fixtures
cover signed and zero runs, duplicate indices, shared/private overrides,
wrap-around and equal coordinates, independent per-tuple geometry, phantoms,
fractional accumulation, invalid/truncated sources, owner overlap, source-bound
coordinates, cancellation and budgets. The CVT composition fixture asserts saved
CVT bytes, GID identity and advance after reopening; it does not exercise hinting
or prove complete variable-font conformance.

Only static source review, rustfmt formatting/parser checks and whitespace checks
were performed. No Cargo, compiler, build, typecheck, test, font/PDF workload,
rendering, benchmark, browser QA, commit, push or deployment was run. The new
regressions are source assertions, not observed passes.

## Still required

Subsequent source work in `truetype_outline_instance_implementation.md` connects
this decoder to point-preserving outline serialization and internal metric
preparation. The public renderer backend and complete-font API remain separate;
the paragraph below records the boundary at this earlier increment.

The new generic gvar evaluator is not yet the production outline, bounds or
phantom-point backend. Those existing paths still use ttf-parser. Its
duplicate-point boundary is therefore not closed by merely adding this module.
The next stage must decode and serialize explicit glyf points and components,
preserve point/GID identities, rebuild loca/maxp and bounds, and reconcile those
exact serialized outlines with metrics and contour-indexed layout references.

Freezing CVT values alone does not freeze variation-sensitive instructions such
as GETVARIATION or GETINFO, nor composite attachment, transformations or
USE_MY_METRICS hinting semantics. Whole-font hint/name handling, complete CFF2
serialization, broader variation/color formats, public integration and all other
roadmap implementation/runtime gates remain open. No universal-editing or
Acrobat-superiority conclusion follows from this increment.

## Primary format references

Packed point/delta and repeated-index semantics follow
[OpenType common variation formats](https://learn.microsoft.com/en-us/typography/opentype/spec/otvarcommonformats).
The scalar calculation follows the
[OpenType interpolation algorithm](https://learn.microsoft.com/en-us/typography/opentype/spec/otvaroverview).
Per-contour inference and component/phantom distinctions follow
[the gvar specification](https://learn.microsoft.com/en-us/typography/opentype/spec/gvar).
CVT ownership and mandatory embedded peaks follow
[the cvar specification](https://learn.microsoft.com/en-us/typography/opentype/spec/cvar).
These sources establish format semantics, not qualification of this implementation.
