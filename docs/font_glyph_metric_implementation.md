# Per-glyph variable-font metrics — source increment

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This extends the
font-instancing work; it does not finish whole-font generation, the public editor,
the full roadmap or executable qualification.

## Implemented composition

`fonts/font_metric_instance.rs` binds preparation to input bytes, an explicit
face index and selected coordinates. It reuses the collection directory and
canonical outline gateway, retains source GID order, captures default/selected
geometry, and returns an input hash with the staged metrics. Explicit unknown
axes are rejected here rather than receiving descriptor-fallback semantics.
Non-variable faces ignore and report inapplicable metric-variation tables.

`fonts/glyph_metric_variations.rs` resolves HVAR/VVAR through the shared variation
store. Scalars are prepared once per table. Implicit advance indices, explicit
format-0 maps, aliased maps, last-entry extension, optional side-bearing/origin
maps and no-variation indices retain their distinct meanings. Missing optional
maps are not confused with required advance data. Invalid indices, versions,
extents, coordinate counts and disallowed long-word rows are errors.

`fonts/glyph_metric_instance.rs` combines those deltas with outline geometry and
explicit gvar phantom deltas. Without HVAR/VVAR it derives advances from both
relevant phantom points, not one point alone. Explicit metric-table deltas are
not added again to phantom-derived values. CFF/CFF2 side bearings use the selected
outline where no explicit leading-bearing map exists. CFF vertical origins and
top bearings are reconciled together; TrueType ignores VORG as specified.

The writer preserves GID order, expands compressed source metric tails when
advances diverge, compresses only identical final advances, rebuilds hhea/vhea
extrema, and excludes outline-less glyphs from ink extrema while keeping their
advances. VORG output uses a deterministic most-frequent default plus sorted
exceptions. Signed fields and advances are rounded and checked without saturation.

MVAR stages first. Per-glyph header rebuilding therefore retains selected-instance
font-wide/caret metrics instead of overwriting them from the original header.
A later failure discards the entire stage. Optional redundant metrics that disagree
with resolved geometry or phantom data produce explicit difference receipts.
They are **not** silently certified as equivalent: the eventual complete-font
transaction must enforce its fidelity policy before saving.

## Source and resource ownership

Source tables stay immutable. Identical source extents share retained allocations.
Capture is bounded to 256 MiB, each HVAR/VVAR table to 64 MiB, and each rebuilt
metric header to 1 MiB. Shared variation-store row/reference/cell limits apply.
Global MVAR output retains its existing 64 MiB staged-table limit. These are
component bounds, not a measured peak-RSS guarantee. Glyph loops, scalar/row
evaluation, copies, hashing and serialization poll cancellation.

CFF decoding errors are distinguished from legitimate empty outlines. For
TrueType, empty loca extents and zero-contour glyphs are recognized explicitly;
an unresolved non-empty program is not converted to blank geometry. The backend
still cannot establish all-empty composite validity through this path, so such
cases can be refused pending structural glyph validation. Raw glyph rewriting,
hint preservation and complete gvar validation remain separate work.

## Evidence and boundaries

28 new regression functions are present and **unexecuted**:

- Twenty transaction cases cover compressed metrics, explicit/implicit maps,
  geometry and phantom deltas, vertical origins, signed rounding, difference
  receipts, malformed data, overflow, cancellation and MVAR/header composition.
- Seven source-bound cases cover actual synthetic-font outlines, coordinates,
  collection face identity, invalid outlines, ignored non-variable metadata,
  metric-table save/reopen assertions and cancellation.
- One CFF2 case combines selected outline geometry with HVAR advance changes.

The isolated metric fixtures are not complete fonts. The source-bound CFF fixture
with synthetic fvar/HVAR metadata tests coordinate plumbing, not conformance of
that combination as a deployable variable-font format. Assertions involving
reopened fonts have not been executed.

Rustfmt formatting/parser checks and Git whitespace checks passed. No compiler,
build, typecheck, tests, PDF processing, rendering, benchmark, deployment,
commit or push was performed.

The entry point is **internal**. The public font preparer still does not expose
non-default whole-font instantiation or editable CFF2. This stage deliberately
does not return a partially converted sfnt and does not remove variation tables.
Whole-font outline serialization, positioning/BASE/GDEF variation freezing,
hints, naming, dependent tables, permissions/signature decisions and public
integration must compose with it before an editable instance is published.
Independent real-font and PDF evidence remains pending along with the rest of
`universal_editor_roadmap_tracking.md`.

## Primary references

Metric mappings follow the OpenType
[HVAR](https://learn.microsoft.com/en-us/typography/opentype/spec/hvar) and
[VVAR](https://learn.microsoft.com/en-us/typography/opentype/spec/vvar) definitions.
Source metric compression and outline relationships follow
[hmtx](https://learn.microsoft.com/en-us/typography/opentype/spec/hmtx).
Vertical-origin ownership and serialization follow
[VORG](https://learn.microsoft.com/en-us/typography/opentype/spec/vorg).
Phantom-delta provenance was traced through
[ttf-parser's gvar source](https://github.com/harfbuzz/ttf-parser/blob/v0.25.1/src/tables/gvar.rs),
with the broader model from
[gvar](https://learn.microsoft.com/en-us/typography/opentype/spec/gvar).
The [FontTools instancer](https://github.com/fonttools/fonttools/blob/main/Lib/fontTools/varLib/instancer/__init__.py)
was inspected as a comparison; its noted vertical-origin gap was not adopted.
