# Physical page breaks - source implementation, not qualification

This increment closes the prior ambiguity where U+000C was preserved as a
mandatory logical line boundary but the layout engines could continue painting
on another frame of the same physical page. It changes source only. No compiler,
test, PDF workload, renderer, benchmark, browser, deployment, commit or push was
run.

## Linked-story contract

`StoryParagraph` now has an additive `page_break_before` policy with `none`,
`next_page`, `next_odd_page` and `next_even_page`. The older `break_before`
continues to mean next approved frame, which is important for columns. A physical
break skips every remaining frame on the current page. It selects the first
approved later frame satisfying the requested parity or creates bounded owned
continuation pages using the existing page budget and transaction.

U+000C has the same physical meaning inside a paragraph. The separator remains
in the zero-advance logical carrier on the source page; following text starts on
a later physical page. Repeated and trailing form feeds materialize every
requested transition, including intentional blank/parity pages. Widow/orphan
rules restart at the author-declared page segment rather than overriding it.
Paragraph-after spacing is applied on the destination of a trailing form feed.

Every transition produces a `StoryPageBreakReceipt` with paragraph identity,
UTF-8 byte offset, source, policy and from/to pages. Receipts participate in the
preview hash and incremental-layout checkpoints. Empty-continuation pruning
protects the complete transition interval. Parity transitions also retain prior
owned continuations whose removal would toggle the destination's one-based page
parity, so a later contraction cannot invalidate an approved odd/even boundary.
Metadata schema 4 is required
when a story contains a physical break. Session status and the browser worker
publish `story_pagination_policy_version: 1`; the browser refuses mismatched
native assets and exposes paragraph page/parity controls plus cursor insertion
of U+000C. Structural three-way merge normalizes the omitted `none` default,
merges independent text/page-policy edits and emits a typed conflict when
branches request competing parity; browser conflict resolution preserves the
same optional-field semantics.

Conflicts fail before mutation. Preserve-layout mode cannot create physical
pages. Next-frame and next-page policies cannot be set together. Keep-together
cannot span U+000C, a trailing form feed cannot also keep with a successor, and
native image captions cannot split their image/caption block. Linked table cells
reject paragraph/form-feed page commands because their safe boundary is the
table row/group paginator.

## Fresh authoring

`FlowDocument::add_paragraph` now advances after a line ending in U+000C and
retains the separator carrier on the preceding page. `FlowPageBreak` and
`add_page_break_to` provide next, next-odd and next-even page creation, including
owned blank parity pages. The existing append transaction rolls all created pages
back on later failure or cancellation.

A `PdfPageBuilder` is intentionally page-local, so its `draw_paragraph` rejects
U+000C instead of painting following text on the wrong page. Authored table cells
also reject U+000C until a row/group-level physical break can preserve grid,
header and fragmentation invariants.

## Unexecuted regression source

The new/changed cases cover same-page column skipping, approved versus generated
parity pages, repeated/trailing form feeds, page budgets, schema/conflict gates,
horizontal and vertical stories, retained save/reopen text, browser capability
gating, fresh-authoring page ownership and table-cell refusal. These functions
were added as source specifications and were not executed.

## Remaining boundary

This physical page-break layer now feeds the typed fresh-authoring section model
described in `authoring_sections_implementation.md`: section geometry,
first/odd/even running masters, page-number fields and PDF PageLabels are wired
through the canonical writer. Imported-PDF section inference, linked-story
section ownership, imported/linked body cross-reference fields and list
continuation rules remain implementation work. Fresh authoring now has deferred
document/section/anchor page fields plus explicit source-bound
footnote reservations/continuations and typed endnote collections; imported and
linked-story note ownership remains separate. Fresh authored tables expose
row-level next/odd/even page commands through their own paginator; imported and
linked-story table command ownership remains separate.
Compilation, bindings, save/reopen, independent
extraction/rendering and corpus behavior remain pending the VPS gate. No
universal-editor or better-than-Acrobat claim follows from this source change.
