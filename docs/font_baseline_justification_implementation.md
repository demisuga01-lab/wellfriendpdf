# Baseline and justification font staging - source increment

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This continues the
whole-font preparation work; it does not complete font generation, public editing
integration, the entire roadmap or executable qualification.

## Implemented source paths

`fonts/base_instance.rs` rebuilds horizontal and vertical baseline owners,
baseline tags and values, scripts, language extents, and feature-specific extents.
Format-3 coordinates use BASE's own variation store, not GDEF's. Format-2 glyph
and contour-point references remain intact and are reported. Packed pixel-device
adjustments are preserved separately. Null optional values, required owners,
ordered tags, default indices and baseline counts have distinct handling. All
coordinates resolve before BASE's store is retired and the table is emitted with
a version-1.0 header.

`fonts/jstf_instance.rs` retains script/language identity, extender glyphs, priority
order and all eight external lookup modification lists. References are checked
against the source GSUB/GPOS lookup counts; extender IDs are checked against maxp
when that table is available. Embedded maxima use the same value/anchor writer
as GPOS, including pair-set offset ownership. Original extensions are unwrapped,
their programs resolved, and new extension wrappers serialized. Contextual,
nested-extension and inconsistent extension programs are rejected. Structural
owners are allocated before the large programs, and identical source programs
share one generated target.

`font_layout_instance::freeze` now combines GSUB/GPOS selection, GPOS positioning,
JSTF, GDEF and BASE. GPOS and JSTF complete before GDEF is rebuilt without its
shared variation store. A former version-1.3 GDEF becomes version 1.2, retaining
mark-set support. A failure in any component returns no combined stage; source
tables remain immutable. Store-retirement receipts replace the previous
BASE/JSTF-unstaged list. This supersedes the store-retention and unstaged-table
status in the preceding positioning increment.

The existing source-bound metric preparation already calls this combined stage,
so it uses the same selected face and normalized coordinates as outline geometry
and HVAR/VVAR/MVAR evaluation. It still does not publish a partially instanced font.

## Resource behavior

The BASE resolver is separate from the shared GDEF/GPOS/JSTF resolver, including
its index cache: identical outer/inner numbers do not alias across stores.
Each retains the prior one-million structural-unit, four-million evaluated-cell
and 65,536 cached-index limits. Individual source/generated tables are bounded
at 64 MiB. Existing source-capture limits still apply; these are component limits,
not measured whole-process memory guarantees. Traversal, copying, evaluation
and publication poll cancellation.

Immutable leaf, GPOS-anchor and BASE-coordinate relocation caches now use binary
search over append-ordered addresses. Repeated owner-local cloning no longer
requires a linear scan through every previous copy of that same source leaf.
No performance benchmark was run.

## Evidence and remaining work

Twenty-two added regression functions are unexecuted. They cover distinct store
ownership, both baseline directions, script/language/feature extents, aliases,
point/device preservation, nulls and invalid defaults/counts, JSTF priorities,
all modification lists, extensions, pair/cursive/mark programs, source bounds,
atomic failure, cancellation, far source offsets and relocation-cache reuse.
A font-reopening fixture asserts preserved BASE/JSTF bytes and actual shaped
GPOS advances. That is regression source, not an observed result. Two earlier
regressions were updated for the now-implemented shared-store retirement.

Rustfmt formatting/parser checks, static source review and whitespace checks are
the only checks performed. No compiler, Cargo command, build, typecheck, test,
font/PDF workload, rendering, benchmark, browser QA, commit, push or deployment
was run. Compilation and every runtime/corpus gate remain unverified.

This is serialization of font-authored baseline/justification data, not a new
application-level justification engine or a claim that the editor now applies
every font suggestion. Outline/point-identity reconciliation, hint and name
serialization, general 16-bit graph packing/subtable splitting, wider cross-table
validation, public complete-font preparation, and the rest of the editor/rendering
roadmap remain open. Opaque source blocks retained elsewhere are not sanitized
by retiring these variation stores.

The separate BASE store and coordinate-owner formats follow the
[OpenType BASE specification](https://learn.microsoft.com/en-us/typography/opentype/spec/base).
Shared GDEF ownership, priority/modification lists and permitted embedded lookup
types follow the [OpenType JSTF specification](https://learn.microsoft.com/en-us/typography/opentype/spec/jstf).
These references establish format semantics, not implementation qualification.
