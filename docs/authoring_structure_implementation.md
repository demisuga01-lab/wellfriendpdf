# Fresh-authoring structure tree - source implementation, not qualification

This increment adds a canonical structure-tree plan for freshly authored
paragraph, heading, list, figure, table, table-of-contents and index content. No
compiler, tests, PDF workloads, rendering,
accessibility checker, viewer checks, benchmarks, deployment, commit or push
were run.

## Structure contract

Layout registers stable semantic identities before final pagination. Final
serialization scans the materialized page-command sequence, assigns consecutive
page-local MCIDs and emits marked-content scopes together with a catalog
`/StructTreeRoot`, `/MarkInfo`, page `/StructParents` keys and a `/ParentTree`.
TOC containers use the standard `/TOC` role and their rows use `/TOCI`; index
containers use `/Index` and their rows use `/P`. A row that wraps across pages
retains one structure identity with one MCR kid for each physical marked-content
scope.

Ordinary and deferred-field paragraphs use `/P`. Headings use `/H1` through
`/H6`, with `/H` for levels outside that standard range. Lists form
`/L` -> `/LI` -> `/Lbl` and optional `/LBody` trees; a body that crosses pages
retains one identity. Meaningful images require nonempty alternate text and use
`/Figure` plus `/Alt`. The legacy no-alt image method is explicitly serialized
as a decorative `/Artifact` rather than untagged content.

Flowed tables form `/Table` -> `/Caption`, `/THead` or `/TBody` -> `/TR` ->
`/TH` or `/TD` trees.
Column and caller-declared row headers carry table-owner `/Scope` attributes and
stable IDs in the balanced IDTree. Data cells name their applicable column and
row header IDs through `/Headers`; optional summaries stay on `/Table`. Split
body cells retain one identity across page fragments; only the first header
occurrence is semantic, while repeated visual headers and cell backgrounds
remain artifacts. Empty cells are represented as intentionally empty structural
cells rather than fake text. Horizontal merged cells publish `/ColSpan`; a
spanning data cell unions unique header IDs from every grid column it covers.
Typed TH/TD elements may additionally carry a bounded, paired `/WFTableID` and
`/WFCellID`. The same UTF-16 identities are repeated in every corresponding BDC
property dictionary, so a continued cell has one logical owner across pages and
duplicated visible values never become mutation identity. The builder rejects
partial, malformed or duplicate ownership pairs. Each typed-cell BDC also
publishes four finite `/WFLeft`, `/WFBottom`, `/WFRight` and `/WFTop` values for
the exact inner content rectangle. A typed owner cannot serialize through the
ordinary unbounded structure command, and a bounded typed command cannot target
an ordinary structure element. The rectangle must also stay inside its authored
page rather than merely being finite and nonempty.

Generated anchor-page links retain the owning row identity through deferred
field materialization. Each annotation receives a unique `/StructParent`, the
ParentTree maps that key to its row element, and the row receives an `/OBJR`
with explicit annotation and page references. Page-content and annotation keys
occupy disjoint checked ranges.

Structure registration is part of the same append transaction as layout. A
failure restores both the registry and its next identity. General front-matter
staging seeds disjoint identities, then places successful front roots before
body roots so top-level structure order follows physical page order.

The writer rejects unknown, nested, unbalanced or artifact-intersecting
structure scopes, leaf elements with neither marked content nor an annotation,
invalid annotation ownership and non-unique/non-contiguous annotation parent
keys. Once any semantic structure exists, every page-paint command must be
owned by a structure scope or an artifact scope; mixed silently unowned content
fails closed. ParentTree entries are sorted into bounded 64-entry indirect leaves and
branches, with checked `/Limits`, rather than growing one unbounded flat array.
An explicit document language is parsed with bounded RFC 5646/BCP 47 grammar,
including grandfathered, extension and private-use sequences, then published as
catalog `/Lang`; no language is guessed from content.

## Unexecuted regression source

Fourteen source cases cover TOC roles and balanced scopes, serialized tree/MCID and
annotation ownership tokens, one index row spanning multiple pages, transaction
rollback, front-before-body root ordering and a multi-leaf ParentTree. The
remaining cases cover cross-page paragraphs, heading roles, list hierarchy and
figure/alternate-text versus decorative-artifact behavior, plus split table-cell
ownership, typed-cell ownership uniqueness and repeated-header exclusion, plus document-language validation and
publication, exact `/Span`/`/Reference` marker ownership with reciprocal `/Ref`
relationships, stable footnote-body `/Note` materialization and rejection of
unowned painting commands. The existing field
annotation case also checks `/StructParent`. They were added but not run.

## Remaining boundary

This is not a PDF/UA conformance claim. Footnote and endnote bodies use unique-ID
`/Note` elements backed by a bounded balanced StructTreeRoot `/IDTree`. Exact
body marker ranges own `/Reference` elements and reciprocal `/Ref`
relationships. Marker partitioning reuses whole-line shaping, carries each
logical span once and fails closed when a boundary divides a shaping cluster.
Tables publish `/RowSpan` and retain one cell identity across fitted or
line-fragmented cross-row blocks. Multi-level header-group inference and rich
cell content remain open. It does not add per-span language,
namespaces or general rich structure attributes; it does not
infer a structure tree for imported PDFs.
Independent structure validation, assistive-technology checks, bindings and all
runtime qualification remain open.
