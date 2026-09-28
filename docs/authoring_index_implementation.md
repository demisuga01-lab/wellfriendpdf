# Authored document index - source implementation, not qualification

This increment adds a typed back-of-document index to fresh authoring. No
compiler, tests, PDF workloads, rendering, viewer checks, benchmarks,
deployment, commit or push were run.

## Entry and ordering contract

`PdfIndexEntry` owns a display term, existing authored-anchor occurrences,
optional `see`/`see also` references and nested subterms. The complete tree is
validated before page mutation and bounded to 100,000 entries, 128 levels and
100,000 emitted links. Duplicate sibling terms, anchors and cross-references
fail closed.

Ordering can preserve authored order, use deterministic Unicode-scalar
lowercase ordering, or require caller-supplied explicit collation keys. The last
mode lets an application provide ICU/CLDR locale sort keys without pretending
that the engine's deterministic scalar order is linguistic collation.

Every page occurrence must reference an already-declared anchor. Occurrences
are ordered by physical page and top-to-bottom position. Multiple occurrences
on one physical page collapse to the uppermost destination. This deliberate
back-of-document rule makes ordering stable before index pages are appended.
Consecutive physical pages collapse into a range at a configurable minimum
length. Section-number ranges form only inside the same section so a numbering
reset can never produce a misleading cross-section range. Both endpoints remain
clickable and the report retains every covered source occurrence.

## Layout and navigation contract

Each hierarchy level receives checked physical indentation and flows through a
private explicit-region variant of the canonical prepared field-paragraph
layout. Page values retain declared fixed capacities, resolve during final
materialization and create shaped-range `/Link` annotations to named
destinations. Section-number values are optional. Long rows may wrap and
paginate; inter-row spacing never creates a trailing blank page.

Final serialization emits a standard `/Index` container with `/P` row elements.
Every physical row fragment receives a page-local MCID while retaining one row
identity across pages. Clickable occurrence annotations receive `/StructParent`
keys, ParentTree ownership and row-level `/OBJR` relationships.

The entire index is one append transaction. Layout, shaping, cancellation or
field-planning failure restores pages, commands, cursor and field identity.
Reports disclose term hierarchy, retained anchor identities, deduplicated source
pages and every physical output page occupied by each row.

## Unexecuted regression source

Six index source cases cover deterministic sorting and same-page deduplication,
explicit collation keys, nested indentation with cross-references, and
pre-mutation refusal for an unresolved occurrence, plus page-range compression
with clickable endpoints and section-reset range isolation. Fourteen shared
authored-structure cases cover structure-tree emission and ownership. They were
added but not run.

## Remaining boundary

This is an explicit back-of-document index, not automatic semantic-term
extraction. Locale collation data must be supplied as explicit keys. Subentry
carry headings, richer index attributes and document-term semantics,
front-matter indexes, imported-PDF occurrence ownership, managed bindings and
runtime qualification remain open.
