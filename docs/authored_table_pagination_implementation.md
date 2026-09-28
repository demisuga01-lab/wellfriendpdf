# Authored table row pagination - source implementation, not qualification

This increment closes the simple authoring API's oversized text-row refusal
route with measured, source-preserving row fragmentation. It is separate from
the imported-document/story table model; it does not replace that model's
rowspan, typed-value, tagged-owner or opaque-object handling.

## Source implementation

`TableBuilder` and `FlowDocument` now share a prepared-row writer in
`authoring_tables.rs`. A cell is prepared once per table operation at its actual
inner column width. Its complete logical paragraph supplies the measured lines,
bidi/joining context, exact registered font asset and any mixed-font run plans.
Horizontal column spans are first-class logical cells. Validation maps each
caller cell onto consecutive fixed-grid columns, rejects zero/overflowing spans,
and inserts explicit trailing empty cells for uncovered columns. Preparation
uses the sum of the occupied column widths; painting emits one background/border
rectangle across that exact range rather than fake duplicated cells.
Continuation pages consume ranges of those same lines. They do not clip a tall
row, slice arbitrary text bytes or treat each continuation as a fresh paragraph.

Each prepared cell stores monotone height and UTF-8 prefix indexes. Height
selection includes shaped ascent/descent, chosen line spacing and per-fragment
padding. Each fragment carries independent cell line ranges: a short cell ends
once, while its taller neighbor continues. Completed cells receive the
continuation background/border but do not repeat their text. Empty and
hard-break/control-only cells retain the existing exact logical-carrier rules.
Per-cell fill, border, alignment, font size and font assets are retained.

The default `TableRowSplitPolicy::Lines` keeps any row that fits a fresh page
whole, moving it when necessary. Only oversized rows split. Callers can choose
minimum nonfinal-fragment/final line counts or explicitly retain
`KeepTogether`. A header remains whole and reserves space with the first body
fragment; it is never left by itself at the end of a page. It repeats on each
continuation page. Header-only and empty tables have explicit paths.

An authored `TableRow` can also own a typed `page_break_before` policy for the
next, next odd or next even physical page. The flow paginator closes the prior
page, inserts any required parity blank in the active section, repeats the
header only on the final destination and begins the row there. A strict break
always advances to a later page even when the current row cursor is already at
the top. Single-page `draw_on_page` rejects this policy before painting.

This changes the default behavior of formerly rejected oversized rows. Callers
that require the previous refusal must choose `KeepTogether`. Single-page
`draw_on_page` retains its explicit whole-table anchoring contract; it shares
preparation/emission but does not invent continuation pages. Use flow authoring
when page-capacity enforcement and continuation are required.

## Feasible continuation algorithm

A greedy largest-fitting prefix is insufficient with unequal line heights and
minimum final-line constraints. Before fragmentation, each cell computes
backward suffix feasibility for the fresh-page capacity after its repeated
header and padding. The recurrence distinguishes a valid final group from a
nonfinal group followed by a viable suffix. Monotone height lookup and nearest
viable-boundary indexes make preprocessing O(n log n) time and O(n) storage per
cell. A current-page cut selects the largest fitting viable boundary, including
when the first page has less available space than later pages.

For example, line heights `[1,1,1,9,1,1]`, capacity 10 and two-line minima require
`[1,1] / [1,9] / [1,1]`. Taking three lines first strands the remainder. A source
regression covers this case; another compares the recurrence with exhaustive
small partition enumeration. Neither was executed.

All active cells advance at a selected cut; completed cells may remain empty.
An impossible full-page line/minimum/header combination returns an explicit
error instead of allocating pages indefinitely. These are fixed-width authored
columns on equal-size flow pages, not a general variable-frame table paginator.

## Transactions, reports and resource handling

`FlowDocument::add_table_with_report` appends atomically and returns
`TableFlowReport`. Every `TableFragmentInfo` discloses the one-based page,
zero-based body-row index (or header), repeated-header flag, top/height,
per-cell original UTF-8 ranges and whether the row continues. Each cell receipt
also binds the caller's source-cell index (or an implicit empty cell), starting
grid column and column span, so a merged cell is not misreported as independent
column text. The report also
records pages added. `add_table` keeps its existing fluent return type and uses
the same transaction.

Every forced transition also returns `TableRowPageBreakInfo`: the zero-based
source row, requested policy, one-based from/to pages and number of allocated
pages including parity blanks. These receipts and the final `added_pages` total
are created inside the same append transaction as row content.

The append transaction restores existing page command lengths, added pages,
active page and cursor on any later shaping, geometry, policy, cancellation or
budget failure. Prepared rows are released as they finish. New pages are builder
objects, not repeated reopening/serialization of a growing PDF. Final PDF
serialization still uses the canonical authoring writer.

When a preceding footnoted paragraph reserves the current page bottom, table
availability and fragment bounds use that effective bottom rather than the raw
margin. A table therefore moves or fragments instead of overpainting note text.

Cancellation is polled during preparation, line indexing, feasibility,
fragmentation, command emission and allocation. Table-wide limits are 1,000,000
prepared lines, 1,000,000 declared cells, 64 MiB of source text and 100,000 emitted
fragments; output cell occurrences and output lines have separate 1,000,000
limits so repeated headers and completed-cell continuation backgrounds cannot
multiply a small source into unbounded commands/reports. Existing column/row
limits also apply. The limits are resource guards,
not performance or concurrency qualifications.

Inspection also removed the old one-point minimum-width clamp: valid sub-point
inner cells now use their actual width and reject a glyph that cannot fit.
Nonfinite/negative border widths and invalid flow state are checked before
emission. Artifact marker balance is checked during PDF serialization.

## Repeated headings and semantics

The first header remains ordinary content; repeated headers and decorative
cell rectangles receive artifact marked-content boundaries. This follows the
PDF Association's [Tagged PDF Best Practice Guide, table creation guidance](https://pdfa.org/download-area/publications/Tagged-PDF-Best-Practice-Guide.pdf).
ActualText/Unicode information inside repeated visual headers is retained.
Ordinary extraction tools may still return artifact text; this is not a promise
to suppress headers from every search or copy interface.

Flow tables now additionally register `/Table`, `/Caption`, `/THead`, `/TBody`,
`/TR`, `/TH` and `/TD` structure elements. Dedicated header rows default to `/Scope /Column`;
body cells may declare row or combined scope. Every TH receives a stable ID in
the structure IDTree and each TD names its applicable row/column IDs through
`/Headers`. A horizontal merge publishes `/ColSpan`; a data merge unions and
deduplicates the applicable headers from every occupied grid column. Optional
table summaries are bounded structure metadata. Split body
cells retain one identity across fragments, empty cells remain explicit empty
structural cells, and repeated headers stay artifacts. Visible captions share
the canonical shaping path and remain with a feasible first fragment; otherwise
the transaction refuses. The page-local `draw_on_page` path paints captions but
has no document structure registry and remains untagged. These changes do not
establish PDF/UA conformance. Oversized cross-row-span continuation, inferred
multi-level header groups and rich cell content remain open. The imported-story tagging
implementation has its own separate ownership contract.

Fresh authored cells can now bind stable typed identities to the existing
`typed_tables` value graph. Exact decimal coefficients/scales, checked
add/subtract/multiply/sum, dependency ordering, missing-reference and cycle
rejection, and no-silent-rounding display rules run before any page mutation.
Only the private resolved table clone receives evaluated text. Flow reports
return the complete identity-to-painted-string map. Repeatable headers cannot
own formulas. Successful flow tables retain a bounded catalog-owned JSON stream
containing stable table/cell IDs, row/column spans, exact values/formulas and
evaluated strings. The public loader independently reevaluates the graph and
rejects duplicate identities, malformed limits or stored-result drift. Registry
insertion participates in append rollback. Each typed TD now also publishes the
paired table/cell identity on its structure element and every marked-content
fragment. Each typed marked-content scope also records its finite inner-cell
content rectangle. The source inspector reconstructs logical values through
direct `/ActualText` provenance (rather than matching visible words), retains a
zero-width text operand for an empty typed cell, rejects registry/source drift
or inconsistent per-fragment regions and returns exact page-logical ranges,
source text state and content rectangles. New registry entries retain the
authored font size, line spacing, alignment, device-colour paint, padding,
border and simple-section continuation geometry that cannot be recovered
uniquely from painted glyphs. A revision-bound mutation transaction can replace updated
values, reevaluate dependents, dynamically rebind every exact owner, prepare one
paragraph and distribute its complete shaped lines across all existing owned
rectangles before changing any bytes. Each fragment rewrite is injected at its
first original owner operand inside the existing MCID; obsolete line operands
are removed, so the edit neither appends a page-level overlay nor changes its
paint order. It then replaces the registry object, reopens and inspects the
result before publishing it. Both authored serialization and reopened inspection
reject a rectangle outside the effective page box, so modified private
properties cannot authorize arbitrary page-wide painting. Clearing the value
retains exactly one zero-width text operand in every typed owner, allowing a
later exact-owner refill. Values that use fewer fragments leave addressable
empty owners. When any cell in a fully typed body row that exactly tiles the
retained columns outgrows those owners, the transaction now plans against the
retained full-page cell
rectangle, creates every sibling cell carrier/grid rectangle in an ordered
continuation batch, inserts it once through the canonical page-tree writer,
appends each actual new MCID to its original TH/TD structure owner and
rebuilds/validates the ParentTree before writing replacement lines. New pages
reuse each cell's original Standard-14 or embedded font and preserve row-header
scope plus column spans. A resolvable repeatable header retains its exact text,
column spans, measured height, alignment, line spacing, device paint and saved-
PDF font identity. Every continuation reserves that height and repaints the
header as an artifact before the body carriers, without duplicating the original
TH ownership. Standard-14 faces resolve directly; embedded faces resolve by the
exact authored base name only when every matching saved resource contains the
same program. Contextual fallback stacks, missing metadata and ambiguous font
programs fail closed rather than changing typography. Every allocated page
receives a private, reopen-checked
continuation provenance dictionary containing the exact table identity, body
row and expected row-cell count. A later shrink transaction can therefore consider only
pages created by this table; matching geometry or apparent visual emptiness is
never ownership evidence. Opt-in contraction hashes every decoded byte outside
the exact typed-cell scopes, requires all expected cell operands to be empty,
refuses annotations, additional page features, changed static paint and live
incoming page references, detaches only the matching page MCRs, rebuilds the
ParentTree and then calls the canonical page-tree pruning writer. Surviving page
labels and shifted page numbers follow that writer's guarded transaction; the
report separates removed pages from retained pages and reasons. Page markers
are scanned once per page and grouped by retained row, not once per cell. A
multi-row table is supported when the target row has complete typed text/grid
ownership and every downstream row already begins on a page strictly after the
target row's final page; the canonical insertion then shifts those later pages
without moving their contents independently. Contiguous later rows that still
share the target's final page now take a separate exact-source relocation path:
their page-private marked-content and grid artifact ranges are removed, their
MCRs are detached, and one newly inserted page packs the rows at their retained
heights below the repeated header. Empty carriers on that page are attached to
the original TH/TD owners and refilled from the registry's exact evaluated text
and saved-PDF fonts before the output postcondition runs. Unrelated page bytes,
typed owners and paint remain in place. The operation parser also requires only
structural closers after the displaced ranges; later non-table paint/text stays
an explicit wider document-reflow case instead of being silently reordered. The
origin page now retains a versioned relocation receipt containing the target
row, an indirect destination-page identity, every displaced cell's original
rectangle/content region/font resource and a digest over all destination bytes
outside the owned cell scopes. On opt-in shrink, backward compaction proceeds
only when every target continuation is empty, that digest and the canonical
incoming-reference guard still pass, every retained value fits its exact
original font/region and the origin remains the structural page tail. The
transaction appends isolated native grid/text carriers at the original paint
position, allocates fresh page-local MCIDs, reattaches the original TH/TD owners,
removes the receipt and destination MCRs, then deletes the relocation page with
the canonical page-tree writer. Intermediate revisions are never returned.
New registries retain one explicit
row-break entry per body row, including a known absence. If any downstream row
owns a next-odd/next-even break, an odd-sized insertion is refused so its
physical parity cannot silently change; older multi-row registries without that
provenance are not upgraded by guesswork. This path fails closed for section
masters, mirrored margins, reserved
footnotes, nondefault fragment-line minima, unsupported header font contracts,
partially typed/mixed rows, row-spanning target cells or downstream rows already
split across multiple pages rather than fabricating their appearance. Multiple
receipts now rebind after each rewrite and nested receipts compact from child to
parent. Before a child mutates its origin, the transaction verifies that the
same page is still the parent's exact destination; only a verified child-owned
rewrite may refresh that parent's digest. General multi-row contraction and
row-height changes still require a broader row-wide retained pagination
transaction; a default-page region, concatenation or overpainting would not be
a valid fix.
Older v1 registries that lack layout/paint/pagination metadata remain readable
but cannot authorize cross-fragment layout or growth.
Page-local drawing still cannot retain this document metadata.

Fresh retained typed tables now also wrap each body grid/fill rectangle in a
private Artifact property dictionary containing the table identity and exact
row, column, row-span and column-span coordinates. This marker is deliberately
named `WFGridTableID`, not the typed text carrier's paired `WFTableID` /
`WFCellID`, so text extraction cannot reinterpret static paint as logical cell
content. The public grid-paint inspector reopens every bounded page stream,
uses the canonical inline-image-safe operation parser, validates one finite
painted rectangle per scope, returns the exact decoded-stream range and reports
whether distinct paint topology equals the complete typed-cell topology.
Duplicate properties/ranges, nested content, unsupported painting, opaque
streams and out-of-page geometry fail closed. This establishes the source
ownership prerequisite for relocating later rows and their paint. The separated-
page and same-page transactions above use it to prove row-local text/grid
correspondence before any source range is moved.

The current source also accepts `TableCell::row_span`. A table-wide occupancy
planner validates positive bounded spans, skips slots owned by cells that began
in earlier rows, creates only genuinely uncovered implicit cells and rejects
header-to-body spans or page breaks through an active merge. An acyclic
row-boundary constraint graph computes the deterministic minimum height satisfying
every ordinary row minimum and every spanning cell's measured content. Page-local
painting emits one rectangle per logical cell. Flow pagination treats boundaries
crossed by spans as connected blocks, moves fitting blocks atomically, repeats the
header only at safe page boundaries and reports the original row/column topology.
`/RowSpan` and row-header coverage across every occupied row use the same plan.
Connected blocks taller than a fresh page use the same source-level cut planner:
it searches descending row/line boundaries, refuses cuts through measured lines
and enforces nonfinal/final line minima while allowing legitimate geometry-only
rowspan progress. Each page paints the exact cell/segment intersection, emits
only complete lines owned by that slice, reuses the logical cell's structure
identity and reports monotone UTF-8 ranges. Repeated headers are inserted only
after the cut is proven feasible. A line/minimum combination with no safe cut
still fails atomically rather than clipping or looping.

## Unexecuted regressions and verification boundary

Forty-one source regression functions cover multi-page exact logical content and
save/reopen assertions, repeated artifact headers, short/empty cell completion,
whole-row moves, explicit no-split rollback, feasible unequal-height partitions,
an exhaustive small partition oracle, line minima, impossible capacities,
failure after earlier pages were appended, mixed-font RTL, control carriers,
style/font identity, header-only/no-op tables, narrow/invalid geometry,
cancellation/budgets, column-span geometry/topology/source partitioning,
cross-row occupancy planning, safe oversized-block cut selection, exact typed
dependency evaluation/cycle rollback and
exact owner/source inspection, zero-width empty-cell provenance, bounded
content-region ownership, clear/reopen/refill owner retention, continued-cell
collapse, cross-fragment redistribution without appended overlay streams,
single-cell and fully typed multi-cell row continuation allocation with reopened
structure ownership, empty sibling carriers, provenance-bound empty-page
contraction and refusal to remove continuation pages with added page state,
retained repeatable-header reconstruction across grow/reopen/shrink/prune,
single-row and row-spanning multi-row grid-paint ownership/reopen inspection,
separated multi-row growth before downstream rows plus row-scoped shrink/prune,
and exact removal/rebinding with retained-height packing of multiple same-page
downstream rows followed by receipt-bound backward compaction to their original
page geometry, nested child-before-parent receipt compaction, pre-mutation parent
digest refusal when that shared page was externally changed, and explicit
retention when destination page state is added,
post-reopen dependent recalculation and unbalanced artifact rejection.
Added row-command cases
cover odd-page parity allocation, repeated-header placement, page-local refusal
and rollback after a forced transition. The older oversized-row refusal fixture
now explicitly chooses `KeepTogether`.

Only rustfmt formatting/parser checks, read-only source inspection and Git
whitespace checks were performed. No Cargo, compiler, build, tests, PDF workloads,
rendering, benchmarks, binding/browser execution, commits, pushes or deployments
were run. Save/reopen code in regression files is not runtime evidence.

## Remaining roadmap

Fresh-document authored tables still lack embedded figures
and nested tables, rich multi-block cells, row/section keep-group policies,
footnotes, split-row/general multi-row or mixed-cell post-reopen growth/shrink,
general row-height contraction,
non-page fragment compaction, general section-master continuation and complete
accessibility relationships.
Mixed page sizes/column widths and general table/document-wide constraint
optimization require the wider story/table model rather than extending this
fixed-column API implicitly. C, Java, .NET, Python and WASM JSON surfaces are
wired in source, including optional approved font bytes where applicable;
typed request models, browser controls and runtime binding proof remain open.
All current-revision build, rendering, extraction,
performance and difficult-document qualification remains pending.

The universal editor/rendering roadmap remains active and incomplete.
