# Exact linear CFF2 compatibility - source implementation

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This extends the
full editor/rendering roadmap; it does not complete it. No builds, tests, font
or PDF workloads, benchmarks, rendering, browser QA or deployment were run.

## Problem addressed

The previous selective-hint route used a floating-point topology model even
when a glyph contained only straight edges. A tolerance-sensitive decision could
merge near contacts or small gaps before deciding whether to keep the original
hinted program. A saved-program area comparison cannot independently validate
that classification if it uses the same numerical engine.

The new `fonts/cff2_linear_contours.rs` classifies the expanded linear subset
using exact fixed-point predicates. It does not flatten cubic curves or replace
the general curve union engine. Curved and mixed line/curve glyphs retain the
declared numerical route and its existing limits.

## Domain and algorithm

The classifier consumes only the already-expanded, decoded Type 2 program.
Coordinates must be finite, within +/-1,048,576 design units, and exactly on the
1/65536 grid. They are scaled to signed integers **without rounding**. Each
coordinate has magnitude at most 2^36. Edge differences have magnitude at most
2^37, so an orientation determinant has magnitude below 2^75. A 4096-edge signed
area sum has magnitude at most 2^85. Signed 128-bit integer arithmetic therefore covers this
declared domain without overflow or floating-point sign ambiguity.

The source implementation:

1. Retains closed rings and source order. Zero-length edges have no geometric
   contribution; their original program bytes are still retained on success.
2. Sorts edges by minimum x and performs exact candidate-contact checks. Adjacent
   forward collinear edges are allowed. Backtracking, nonadjacent intersections,
   coincident intervals, T-junctions and contour-to-contour touches require the
   reconstruction route. This is a sufficient preservation condition, not a
   complete characterization of every CFF1-valid touching boundary.
3. After excluding contacts, computes each simple ring's external winding with
   exact half-open ray crossings. Its orientation determines the internal
   winding. Each boundary must separate zero winding from +1 or -1. Alternating
   holes/islands and disjoint reversed contours can retain their programs;
   redundant same-winding nested ink cannot.
4. Returns compatible / needs reconstruction / contains curves. An exact
   needs-reconstruction result cannot be overridden by a later tolerant
   compatibility check. Hint-loss consent can therefore fail before invoking
   the numerical solver.
5. On exact compatibility, keeps the expanded glyph bytes and frozen private
   owner unchanged, without a numerical union or another coordinate
   quantization. Existing complete-font metric, ownership and readback stages
   still apply.

This is an original bounded-integer implementation, not a port of Shewchuk's
adaptive floating-point code. The motivation for avoiding inexact determinant
signs follows [Shewchuk's robust-predicate research](https://www.cs.cmu.edu/~quake/robust.html).
The source coordinate precision and the CFF2/CFF overlap distinction follow the
[OpenType format comparison](https://learn.microsoft.com/en-us/typography/opentype/spec/glyphformatcomparison).
No new dependency, native executable or external conversion service is added.

## Bounds, reports and compatibility

The classifier has a 4096-edge / bounded path-element limit. Existing source
normalization caps still restrict input glyphs to 2048 segments. Candidate-pair
and containment-edge visits are charged to an aggregate 16-million-work budget,
with cancellation in outer loops and every 256 charged visits. Sorting and
individual exact operations are bounded but not instruction-level cancellable.
The numerical solver's separate preflights and existing hard-cancellation gap
are unchanged for rewritten/curved glyphs.

The report now includes `exact_linear_preserved_glyphs` and `exact_linear_work`.
The algorithm descriptor identifies both the fixed-point and curve routes.
These fields default when reading older reports and are optional in browser
typings. Checked default publication advances the native/browser font protocol
to version 4 so older workers cannot silently retain unchecked outlines.

`independently_verified` remains false. Exact arithmetic for this bounded
decision is not an independent renderer comparison, a compiled/tested result,
or a proof of arbitrary PDF/font correctness. The default request without
`cff2_contours` now uses the same exact-linear or bounded numerical classification
in read-only mode. It preserves glyph/private bytes only when compatibility is
established and otherwise refuses with the normalization route required. It
never silently removes hints or contours.

## Regression source and outstanding evidence

Fourteen new regression functions are added but not executed: thirteen predicate
and dispatch cases and one complete-font case. They cover determinant products
differing by one, maximum coordinates, all orientation combinations of nested
contours, interval-oracle rectangle contacts, one-quantum gaps/overlaps,
collinear vertices, duplicate points, backtracking, crossings, T-junctions,
curved dispatch, malformed/off-grid inputs, budgets/cancellation and exact
hinted-font byte retention at a large numerical tolerance. Existing preserved
font, older-report and retained-session save/reopen regression source now also
checks the new receipt fields and bypassed solver counters.

Formatting/parser checks and scoped Git whitespace checks passed. Compilation,
dependency resolution, native/binding/browser execution, adversarial memory and
timing, and independent raster/corpus comparisons remain unexecuted. Curved
compatibility and union are still numerical; rewritten-outline hint transfer,
automatic rehinting, complete font semantics and the full editor/rendering
roadmap remain unfinished.
