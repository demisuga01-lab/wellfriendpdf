# CFF2 static publication — source implementation, not qualification

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This increment
extends the existing public font transaction to variable and non-variable CFF2
sources. It does not complete the editor/rendering roadmap or establish universal
font conversion. No compiler, build, tests, font/PDF workload, rendering,
benchmark, browser QA, commit, push or deployment was run.

## Native publication path

`fonts::font_instance::prepare_font_instance` now accepts CFF2 as well as the
previously supported variable TrueType sources. The request still binds an exact
source SHA, collection face, coordinates, explicit output names and any signature
removal decision. A non-variable CFF2 face uses an empty coordinate map. Unknown
axes, unsupported table owners and embedding restrictions are not bypassed.

`fonts/cff2_instance.rs` writes a complete CID-keyed CFF1 table from the selected
source program. It does not save the existing rendering-only outline projection:

- Every source GID remains the same output GID and CID, including empty glyphs.
  Existing cmap, substitutions and positioning retain that glyph domain.
- Local and global subroutine calls expand using each glyph's actual source
  FontDICT. Inherited and glyph-overridden variation indices keep their roles.
- Private DICT blends are evaluated with the shared region evaluator. Supported
  alignment zones, stem widths, language group and hint parameters are emitted,
  not silently stripped. Resolved stem-snap sets are sorted and deduplicated if
  interpolation changes their order; the report counts these normalizations.
- Stem declarations, hint/counter masks and flex operators are retained. Binary
  mask payloads keep their original hint-bit ownership. Large stem groups split
  with rebased origins, including rounding of the deltas actually serialized.
- Explicit width deltas come from the staged hmtx metrics. A nominal width of
  32768 makes the complete unsigned hmtx range representable by signed Type 2
  operands. Empty glyphs receive widths too.
- Unused FDs are removed. Identical frozen private semantics can share one output
  FD even when their source subroutines differ, because calls have already been
  resolved per source owner. Distinct hint dictionaries are never merged merely
  to fit the output selector domain.

The stack, blend and hint ownership model follows the
[CFF2 specification](https://learn.microsoft.com/en-us/typography/opentype/spec/cff2).
The destination dictionary/INDEX/charset representation follows
[Adobe CFF](https://adobe-type-tools.github.io/font-tech-notes/pdfs/5176.CFF.pdf);
widths, masks and flex emission follow
[Adobe Type 2](https://adobe-type-tools.github.io/font-tech-notes/pdfs/5177.Type2.pdf).
The split-stem origin behavior was cross-checked against the
[FreeType interpreter source](https://github.com/freetype/freetype/blob/master/src/psaux/psintrp.c).
No external converter is invoked and no new dependency was added.

## Whole-font composition and postconditions

The same source-bound metric/layout stages feed TrueType and CFF2 publication.
Selected HVAR/VVAR/MVAR values, vertical origins, supported layout variations,
names and STAT handling are composed before final serialization. The new CFF
writer retains staged `head`/`post` updates, rather than overwriting MVAR changes
with original table bytes. Output uses OTTO, maxp 0.5 and post 3.0. CFF2 and
completed variation owners are retired; the source buffer remains immutable.

Before asset publication, the implementation checks:

- CFF1 reparse, glyph count and every GID/CID mapping;
- standalone/static sfnt identity and UPEM;
- each horizontal/vertical metric and staged VORG origin;
- each reopened CFF1 outline bound against selected source geometry;
- every staged/preserved table's exact owner, length and SHA-256 receipt, except
  the final writer's `head.checkSumAdjustment` field.

These are implemented structural postconditions, not checks executed during
this increment. Matching bounds are not proof of identical curves or pixels.
The shared public transaction still refuses unresolved layout point references,
remaining variation-store ownership and unapproved redundant-metric differences.

## Editing, authoring and browser integration

The returned standalone CFF1 asset goes through existing approved-font,
provider, authoring, shaping, embedding and saved-story paths. Preparation alone
does not change PDF bytes or approve an edit. Generic native/managed session
transports use the existing `prepare_font_instance` command.

Native status and the browser command gate now require
`font_instance_protocol_version: 4` after checked default contour publication.
The local font picker offers explicit static
preparation for CFF2 faces, including those without axes. Ordinary face extraction
still does not convert CFF2 or pin variable coordinates. The picker neither
installs nor downloads fonts and remains disabled for collaborative structural
drafts. Rebuild/deploy matched worker/native artifacts together during later QA.

The report adds `output_outline_format` and a nullable `cff2` record with source
and output FD counts, glyph/hint/mask/blend counts, normalization/work counts and
explicit contour-overlap status. `independently_render_verified` remains false.

## Bounds

- Existing CFF2 parsing limits remain: 32 MiB source table, one million INDEX
  entries, 16 MiB aggregate parsed DICT data and 64 variation axes.
- Publication limits the complete CFF1 table to 32 MiB or the caller's smaller
  output limit, each expanded glyph to 65535 bytes, glyph work to one million
  evaluation/emission steps and aggregate private/glyph expansion to 16 million.
- CFF2 evaluation uses at most 513 operands, ten subroutine nesting levels and
  96 hints. Emitted CFF1 operators use at most 48 operands, including width.
- At most 256 distinct frozen output private dictionaries are supported. A
  larger source FD domain is accepted only when its used semantics coalesce.
- Signed 16.16 operand overflow is an error, not clamping. Offsets and INDEX
  extents are checked; shared private ranges are evaluated once.
- Loops, serialization chunks and postconditions poll cooperative cancellation.
  Native full-font bounds remain 256 MiB; retained-session transport remains
  4 MiB. These are component limits, not measured peak-memory or latency claims.

## Regression source and allowed checks

Twenty-four new regression functions are unexecuted: 23 CFF2 publication cases
and one retained-session case. They cover widths across the unsigned domain,
FD remapping/deduplication, actual nested subroutine owners, private/glyph blends,
stem crossings, masks, split-origin rounding, flex, malformed/unknown semantics,
budgets/cancellation, exact collection selection/signature decisions, selected
metrics/vertical origins, authoring extraction and story checkpoint/reopen/re-edit.
The original browser capability regression advanced to version 2; subsequent
contour changes now require version 4.

Rustfmt formatting/parser checks and whitespace inspection are the only local
validation used. They do not establish compilation, regression results,
hint execution, browser behavior, interoperability or independent rendering.

## Remaining implementation and qualification

The default preserve-source mode retains selected source
contours only after a compatibility check, reports `contour_overlaps_checked: true`, and
`contour_overlaps_removed: false`. CFF2 allows overlaps that legacy CFF consumers
need not support identically; see the
[OpenType format comparison](https://learn.microsoft.com/en-us/typography/opentype/spec/glyphformatcomparison).
The follow-up `cff2_contour_normalization_implementation.md` adds opt-in numerical
curve union with explicit hint-loss consent and actual serialized-path checks.
Exact hint-preserving normalization and independent fidelity remain open. The reference
[FontTools converter](https://github.com/fonttools/fonttools/blob/main/Lib/fontTools/cffLib/CFF2ToCFF.py)
also separates conversion from optional overlap removal; that is not evidence
that this SDK's output is visually qualified.

Unknown operators/table owners, newer variation formats, color/AAT/private
semantics, oversized expanded programs, broader point-reference ownership and
general hint sanitization remain bounded or unsupported. This is not a full
font sanitizer. CFF hint behavior, complex real fonts, overlaps, independent
rasterizers, bindings and repeated PDF edits require executable qualification.
The full `universal_editor_roadmap_tracking.md` objective remains open.
