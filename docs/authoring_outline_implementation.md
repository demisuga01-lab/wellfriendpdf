# Fresh-authoring document outlines - source implementation, not qualification

This increment adds a native hierarchical PDF outline to fresh authoring. It is
source work only: no compiler, test, PDF workload, renderer, viewer, benchmark,
browser, deployment, commit or push was run.

## Authority and targets

`PdfBuilder::set_outline` and `FlowDocument::set_outline` atomically replace a
typed tree of `PdfOutlineEntry` values. Every item carries a bounded single-line
title, a named authored anchor, an open/closed state and ordered children.
Targets may be forward references during layout, but final serialization fails
unless every target is a declared anchor in the same document. No title or
geometry inference is performed.

`FlowDocument::add_outlined_heading` is the explicit capture route. It binds an
anchor at the pre-heading cursor, paints the existing visible heading style and
appends the corresponding outline item as one operation. Heading levels start at
one, may increase by only one, and may close any number of prior branches.
The anchor is created only after the first line's final destination page is
known, so an automatic page break cannot leave navigation on the preceding
page. Failed font coverage, shaping or pagination removes the new anchor/item
and restores the level stack, cursor, pages and commands. Replacing the outline
resets that capture stack so a stale hierarchy cannot silently attach to a new
tree.

Validation bounds the tree to 100,000 items, 128 levels and 16 KiB per title or
target. Invalid replacement input leaves the previous outline untouched.

## PDF tree contract

Final serialization allocates the outline root and all items in one transaction.
It emits `/Type /Outlines`, `/First`, `/Last`, `/Parent`, `/Prev`, `/Next` and
named `/Dest` relationships using final indirect object identities. Positive and
negative `/Count` values reflect the exact descendants visible under each
open/closed item; the root count reflects only initially visible entries. The
catalog receives `/Outlines` and `/PageMode /UseOutlines` only when a nonempty
outline exists.

The same encoded destination strings feed the separately balanced `/Names`
`/Dests` tree. Object allocation is checked and every traversal polls
cancellation. This is native PDF navigation, not a page-content table of
contents and not an accessibility structure tree.

## Unexecuted regression source

Eleven source cases cover forward targets, hierarchy links and visible counts,
closed branches, unresolved-target atomicity, invalid replacement rollback and
catalog/root publication, automatic heading hierarchy, skipped-level refusal,
layout-failure rollback, explicit-outline stack reset and destination-page-aware
anchor capture. They were added but not executed.

## Remaining boundary

Outline styling, actions other than named internal destinations, inferred
heading capture from arbitrary styled paragraphs, imported-outline
preservation/merging, page-content TOC and
index generation, semantic heading/link tags, managed bindings and all runtime
qualification remain open. This increment does not establish universal editing
or a better-than-Acrobat claim.
