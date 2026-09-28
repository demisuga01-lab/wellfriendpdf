# Painted authored table of contents - source implementation, not qualification

This increment paints the explicit authored outline into ordinary PDF page
content. No compiler, tests, PDF workloads, rendering, viewer checks, benchmarks,
deployment, commit or push were run.

## Layout contract

`FlowDocument::add_table_of_contents` flattens the current typed outline in
preorder and writes every entry in one append-only transaction. Titles retain
their outline hierarchy through checked physical indentation and wrap through
the canonical prepared-paragraph shaping path. A separately shaped fixed-width
page column is aligned to the content area's right edge. The row extent combines
the actual title and page-font ascent/descent, so different title/page styles do
not overlap adjacent rows.

Optional leader tokens are measured through the same font path, repeated only
within the available final-line gap and capped at 4,096 repetitions. Rows move
to later section-owned pages through ordinary flow pagination. Reports disclose
each source title/anchor, hierarchy level, physical output page, wrapped line
count and exact reserved page-column rectangle.

For RTL and alternate publishing conventions the page column may instead occupy
the physical left edge. Title alignment and page-value alignment are independent;
leader placement follows the actual shaped title edge on either side. The page
field's painting padding moves to the appropriate side while link geometry and
logical extraction retain only the raw value.

Exact zero-based outline levels may override title/page typography, absolute
indentation, line height, row gap, title/page alignment and leader behavior.
Overrides inherit every omitted property, reject duplicate level declarations
and validate before any page mutation. A level may request `keep_with_next` or
`keep_with_previous`; the paginator measures the bounded transitive keep chain
with the actual resolved styles and moves it as one unit when it fits an empty
page. A chain larger than a page degrades to ordinary row pagination instead of
making otherwise valid content impossible.

## Deferred navigation contract

Page values are not guessed during TOC layout. Each row creates a private
single-line deferred anchor field with an explicit character capacity. Final
materialization resolves document or section page numbering after all pages
exist, right-aligns the value inside the reserved column and emits the existing
shaped-range native `/Link` annotation to the named destination. Unknown anchors,
capacity overflow, font coverage changes or geometry growth reject output.

The whole TOC is rollback-capable: a failing row restores pages, commands,
cursor, field-plan identity and structure identity. Final serialization emits a
standard `/TOC` container, `/TOCI` rows, page-local marked-content IDs and
ParentTree ownership. Clickable page annotations receive `/StructParent`
ownership and row-level `/OBJR` relationships. This remains explicit authored
TOC structure, not inferred heading/list semantics for the source document.

## Unexecuted regression source

Eleven TOC source cases cover forward targets and clickable final values, exact
right-column geometry, nested indentation, transactional geometry failure,
unresolved-target refusal, multi-page row flow and a left-column mixed-direction
case, plus exact-level styling, duplicate-override rollback and bidirectional
keep pagination. Fourteen shared authored-structure cases cover broader structure-tree
emission and ownership. They were added but not run.

## Remaining boundary

Broader orphan constraints, document-heading linkage, list semantics,
cross-reference expressions, imported-outline TOCs, complete PDF/UA semantics,
managed bindings and runtime qualification remain open.
