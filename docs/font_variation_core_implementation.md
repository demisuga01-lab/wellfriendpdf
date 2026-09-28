# Portable font-instancing core — implementation increment

This is prerequisite work toward persisted, whole-font static instances. It is
not a completed instancer, an editable-CFF2 release, or completion of the full
roadmap. Work remains uncommitted over
`27e62db3a1b84804339e65b6025273fd003b3736`.

## Shared variation evaluation

`fonts/variation_store.rs` now owns the source-bound ItemVariationStore parser
and evaluator. Parsed row ranges share immutable source bytes. Repeated delta
queries can prepare each region scalar once, rather than recomputing the entire
region model for each metric. Duplicate subtable offsets share metadata.

The evaluator covers signed 8/16-bit and LONG_WORDS 16/32-bit rows, multiple
axes, null item-data subtables, the no-variation index sentinel and packed
DeltaSetIndexMap formats 0/1 with last-entry extension. It returns interpolated
deltas in the table's integer domain; the consumer remains responsible for the
target field's units and bounds. Integer conversion uses explicit half-up
rounding with checked range, not a saturating cast.

Limits are 64 axes, a 64 MiB store extent, one million unique region references
and rows, and four million declared delta cells. Offsets, row extents, axis
counts and region references are checked; cancellation is polled in parsing,
preparation and evaluation. These are component limits, not a measured RSS or
end-to-end latency guarantee.

The CFF2 decoder **uses this core now**. Its source allocation is shared with the
store rather than copied again. CFF2 still requires region-only data without
delta rows. This integration also handles a null ItemVariationData offset as
no active variation regions, instead of trying to interpret the store header
as a data subtable.

## Layout feature freezing

`fonts/layout_instance.rs` implements GSUB/GPOS FeatureVariations selection and
serialization at already-normalized coordinates. Selection respects inclusive
axis ranges, conjunctive conditions and first-match ordering. Unknown condition
formats or invalid condition axes do not match. An unsupported substitution
version advances to the next candidate. A matching null substitution is a no-op
that stops the search, not permission to choose a later rule.

For a selected substitution, the writer rebuilds script/language and feature
owners while retaining feature and lookup indices. Lookup programs remain in
one exact retained source block, reached through new extension wrappers.
Existing extension wrappers are resolved instead of nesting extension types.
Lookup flags and mark-filtering indices are retained. Registered `size`,
`ss01`–`ss20` and `cv01`–`cv99` feature parameters are relocated with their owners.
The output root is version 1.0, without a reachable FeatureVariations root.

This writer is an **internal instancing primitive**, not yet connected to the
public editable-font preparation operation. It only freezes feature selection;
it does not freeze GPOS/GDEF VariationIndex values. The retained source block
can contain unreachable original metadata, so this is not a sanitizing rewrite.

Output is bounded to 64 MiB. Non-representable rebuilt 16-bit offsets produce an
error, not wraparound. General compact offset-graph packing and unknown non-null
feature-parameter relocation remain open. Large valid layouts can still require
that packing work; these errors are not presented as unsupported PDF semantics.

## Regression source and validation

This increment adds 24 **unexecuted** regression functions:

- 12 for signed rows, long words, multi-axis interpolation, null/sentinel
  semantics, shared metadata, source-relative extents, malformed data, work
  limits, index maps, rounding and cancellation;
- 11 for feature ordering, null substitutions, forward-compatible condition
  handling, exact source retention, direct/extension lookup relocation,
  filtering flags, feature parameters, invalid data and cancellation;
- one CFF2 regression for null item-data blending.

The layout regressions include reopened synthetic-font shaping assertions for
both GSUB glyph selection and GPOS advances. They were **not executed**.
Rustfmt formatting/parser checks and Git whitespace checks passed. No compiler,
build, typecheck, tests, PDF processing, rendering, benchmark, browser workload,
deployment, commit or push was performed.

## Remaining instance work

Whole-font preparation must compose selected coordinates, outline conversion,
metrics/HVAR/VVAR/MVAR, GPOS/GDEF/BASE variable values, hinting, naming and other
retained tables before exposing a prepared editable instance. Coordinate
normalization, including newer cross-axis mapping where supported, must be
consistent across every consumer. The public preparer still refuses CFF2.

The researched HarfBuzz instancer provides an alternative native backend, but
none was added: the engine's current default architecture forbids unsafe code
and system-installed font rendering dependencies. The inspected Fontations
subsetter is not a drop-in complete axis-instancing backend for this task.
This increment adds no dependency, external process, or backend availability
claim. The full roadmap and all executable qualification gates remain active.

## Primary references

The [OpenType variation common formats](https://learn.microsoft.com/en-us/typography/opentype/spec/otvarcommonformats)
define region/delta records, null data and compressed indices. The
[OpenType layout formats](https://learn.microsoft.com/en-us/typography/opentype/spec/chapter2)
define feature selection and extension ownership. Integer rounding follows the
[FontTools rounding implementation](https://github.com/fonttools/fonttools/blob/main/Lib/fontTools/misc/roundTools.py).
The [HarfBuzz subset API](https://harfbuzz.github.io/harfbuzz-hb-subset.html) and
[Fontations subset source](https://github.com/googlefonts/fontations/blob/main/skera/src/lib.rs)
were inspected as backend alternatives, not integrated or executed.
