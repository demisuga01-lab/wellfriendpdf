# Point-preserving TrueType outline staging

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This source increment
connects the preceding tuple decoder to actual glyph-table generation and metric
preparation. It does not complete the public font instancer or the full roadmap.

## Implemented

`fonts/glyf_program.rs` parses source locations, explicit contour points, packed
flags/coordinates, component records, transforms and instruction ranges. Glyph
instructions borrow ranges in one immutable source allocation. Implied on-curve
points are not inserted and source point/GID ordering is not changed. Component
dependencies are checked for cycles, invalid identities and excessive depth.

`fonts/glyf_instance.rs` resolves each glyph's tuples through the shared gvar
evaluator. It rounds accumulated simple-point and component-offset adjustments
once, preserves attachment indices, and assembles geometry in dependency order.
Offsets are adjusted before any component-offset transform. Point attachment uses
the transformed child point; component deltas attached to those point references
are ignored and reported. Empty glyphs and structurally empty composites are
distinct from parse failures and retain their phantom deltas where provided.

The stage serializes points and components in original GID order, compresses
repeated flags and small coordinate differences, retains on/off-curve flags,
contour endpoints, matrices and instruction bytes, and rebuilds glyf, loca, head
and maxp. Short loca is used only when every final offset fits; otherwise the
writer uses long offsets. Control-point bounds, not tight Bezier ink bounds,
drive glyph and global bounds. Outlineless glyphs do not enlarge global bounds.
Composite expanded-point/contour counts, depth and glyph instruction byte maxima
are rebuilt; instruction stack/storage/function resource maxima remain intact.

For derived variable outlines, overlap flags conservatively enable overlap
handling. Unspecified or conflicting component-offset scaling is made explicitly
unscaled, following the recommended default, and each such decision is recorded.
This is a reported normalization, not a claim of identical behavior in every
legacy rasterizer. USE_MY_METRICS and pixel-grid-rounding flags remain intact;
this stage does not simulate their device-scale instruction behavior.

`font_metric_instance::prepare` now uses these exact serialized TrueType points
and the engine's tuple-derived phantom deltas for its metric transaction. It no
longer obtains those TrueType geometry/phantom values from ttf-parser. CFF/CFF2
preparation still follows the preceding canonical outline path. The source
TrueType header remains the base phantom-coordinate basis when its bounds need
repair; the repair is separately reported. The rebuilt head sidebearing flag is
synchronized after the final horizontal metrics are known.

This supersedes the preceding increment's statement that the tuple decoder was
not connected to outline generation inside this internal transaction. It does
not replace the renderer's public/canonical TrueType outline backend, and it does
not itself expose static font bytes through the public font-asset API.

## Bounds and source-only evidence

Original and generated glyf tables are each limited to 64 MiB. The retained
explicit-point total and expanded point cache are each limited to two million
points; expanded per-glyph points/contours must fit maxp. Expansion capacity is
checked before assembly, and dependency depth is limited to 64. The transaction
uses the shared 16-million-unit work budget, including tuple-directory work,
and polls cancellation during parsing, evaluation, assembly and emission. These
are component limits, not a measured process-memory or performance guarantee.

Twenty-five new regression functions are unexecuted. They cover packed signed
coordinates, flags, multiple contours, off-curve identities, instructions on
simple/zero-contour/composite glyphs, duplicate and fractional tuple deltas,
matrix orientation, offset scaling, transformed point attachments, ignored
attachment deltas, empty-glyph phantoms, dependency/GID order, malformed records,
cycles, depth/count/coordinate limits, global/profile bounds and short-to-long
location output. A full internal preparation fixture writes a font, reopens it,
and asserts both changed outline coordinates and matching advances, then prepares
the saved font again. Those assertions have not been executed.

Static source review, rustfmt formatting/parser checks and whitespace checks are
the only validation performed. No compiler, Cargo, build, typecheck, test, font
or PDF workload, rendering, benchmark, browser QA, commit, push or deployment ran.

## Remaining implementation

Follow-up: `truetype_hint_instance_implementation.md` adds the prep fallback,
hint-program inventory and exact TrueType layout point-domain checks. The older
remaining-work statements below describe this outline increment's own boundary;
the follow-up narrows them without claiming complete hint or whole-font closure.

Phantom-point component attachments require finalized phantom/hint semantics
and are explicitly rejected by this stage, rather than treated as contour points
or zero coordinates. Variation-sensitive instructions (including GETVARIATION
and GETINFO), full hint freezing, and device-dependent attachment/grid behavior
remain unfinished. Retained instruction bytes alone do not close those items.
Point/count and signed-coordinate representation limits remain explicit; the
writer will not silently insert points or saturate a value to make it fit.

Cross-table validation of contour-indexed anchors/carets/baselines still needs
completion, although this stage preserves their point identity. Complete CFF2
font serialization, name/style updates, broader variation/color formats, the
public complete-font transaction, renderer integration and the rest of the
editor/rendering roadmap remain open, along with every runtime/corpus gate.

Point/component formats and transforms follow the
[OpenType glyf specification](https://learn.microsoft.com/en-us/typography/opentype/spec/glyf).
Tuple/component semantics follow
[the gvar specification](https://learn.microsoft.com/en-us/typography/opentype/spec/gvar).
Rebuilt profile fields follow
[maxp](https://learn.microsoft.com/en-us/typography/opentype/spec/maxp), while
global bounds and sidebearing flags follow
[head](https://learn.microsoft.com/en-us/typography/opentype/spec/head).
These primary references do not constitute implementation qualification.
