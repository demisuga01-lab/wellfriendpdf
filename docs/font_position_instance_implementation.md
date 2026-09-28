# Variable positioning and caret staging - source increment

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This increment
extends internal whole-font preparation. It does not finish static font
generation, public variable-font editing, the full editor roadmap, or qualification.

Follow-up: `font_baseline_justification_implementation.md` supersedes this
increment's BASE/JSTF gaps and GDEF-store-retention policy. The evidence boundary
remains source-only and unqualified.

## Implemented

`fonts/font_layout_instance.rs` stages GDEF and GPOS together against the same
immutable GDEF variation store. GSUB/GPOS feature selection uses the same selected
coordinates. `font_metric_instance::prepare` now includes this stage for variable
faces, alongside outline geometry and HVAR/VVAR/MVAR metrics. Input hash and face
identity still belong to that outer preparation. Non-variable faces do not start
applying stray VariationIndex data. No partially instanced font is published.

`fonts/gpos_instance.rs` rebuilds SinglePos formats 1/2, pair sets and class-pair
matrices, cursive anchors, mark-to-base, mark-to-ligature and mark-to-mark programs.
Source offsets retain their distinct owners; in particular, pair-set device
offsets are not interpreted relative to the enclosing lookup. Numeric fields
absent from a source value record are introduced when needed to hold a resolved
adjustment. The presence of the second pair record is preserved, even when its
resolved adjustment is zero. Final signed values use checked half-up rounding.

Classic pixel-size Device programs retain their packed bytes and relocated
offsets. VariationIndex programs resolve through the owning store, with null
offsets and the no-variation sentinel handled separately. Shared source values
are read immutably, so an alias cannot cause a delta to be applied twice to the
same copied scalar. Repeated index evaluation is cached for this instance.

`fonts/gdef_instance.rs` rebuilds glyph classes, attachment points, ligature
carets, mark attachment classes and mark glyph sets. Caret format 3 resolves
variable coordinates; format 2 point indices are retained and reported. Rebuilding
owner graphs avoids patching a source record that might also serve another
owner. The original variation-store extent is retained because JSTF may still
reference it. Retention is not a declaration that all variation consumers closed,
nor is this source-data sanitization.

`fonts/layout_instance.rs` can now replace positioning programs while retaining
stable script/language/feature/lookup indices. It unwraps source extensions,
serializes replacement programs behind extension offsets, and retains contextual
dispatch bytes with their self-relative offsets intact. Conflicting inner lookup
types within one rebuilt extension lookup are rejected. Identical source lookup
programs of the same kind share one generated target.

## Bounds and remaining limits

Rebuilt positioning tables are limited to 64 MiB. The resolver caps structural
work at one million charged units, uncached delta evaluation at four million
cells and cached variation indices at 65,536. These are component bounds, not a
measured peak-memory or latency guarantee. Input/output copies, coverage/class
parsing, attachment traversal, row evaluation and serialization poll cancellation.
Existing shared variation-store and font-capture bounds also apply.

Offset overflow is an explicit failure. General 16-bit owner-graph packing and
subtable splitting are not implemented here. Contextual programs retain source
bytes and lookup identities; they are not fully sanitized by this writer. GDEF
point references are preserved, not certified against rewritten outlines. A
complete font transaction must preserve their point identities or resolve them
under an explicit fidelity policy.

BASE and JSTF are reported as unstaged when present. Complete outline/hint/name
serialization, those layout consumers, store retirement, public font preparation,
cross-table validation and the wider editor/renderer roadmap remain open. The
new stage is internal; adding it does not make missing licensed glyphs recoverable
or establish universal PDF editing.

## Evidence boundary

Twenty-six new layout regression functions and one shared-store work-accounting
regression were added, not executed. Source fixtures cover value-record expansion,
rounding, pair owners, class matrices, cursive and all three mark lookups, point
references, classic devices, missing stores, sentinels, aliased carets, GDEF owner
graphs, contextual dispatch, feature selection, extensions, invalid sources,
budgets and cancellation. Font-level assertions cover reopened shaping and joint
metric/layout preparation, not only status fields.

Only rustfmt formatting/parser checks, source inspection and Git whitespace checks
were performed. No Cargo command, compiler, build, typecheck, test, font/PDF
workload, rendered comparison, benchmark, browser QA, commit, push or deployment
was run. Passing parser/whitespace checks is not evidence that this code compiles
or that any regression passes. VPS execution and independent qualification remain
pending.

## Primary references used

The ownership rules for value records and anchors, and the extension-lookup
constraint, follow the [OpenType GPOS specification](https://learn.microsoft.com/en-us/typography/opentype/spec/gpos).
The shared variation store, ligature-caret formats and mark-set offsets follow
the [OpenType GDEF specification](https://learn.microsoft.com/en-us/typography/opentype/spec/gdef).
Device/VariationIndex distinctions follow the [OpenType common layout formats](https://learn.microsoft.com/en-us/typography/opentype/spec/chapter2).
These references specify formats and semantics; they are not execution evidence
for this implementation.
