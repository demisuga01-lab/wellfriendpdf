# Transactional authored front matter - source implementation, not qualification

This increment can prepend arbitrary freshly authored blocks or a convenience
painted table of contents before an already-authored body. No compiler, tests,
PDF workloads, rendering, viewer checks, benchmarks, deployment, commit or push
were run.

## Staging and splice contract

`FlowDocument::prepend_front_matter` receives an explicit first `FlowSection`
and a closure that authors pages into an isolated flow. Paragraphs, tables,
images, section-scoped notes, fields, additional sections and generated TOCs use
their normal APIs. The staging flow begins with copies of the body's fonts, font
stacks and images; newly registered resources are merged only after success.
Anchor, note, deferred-field and authored-structure identities remain
collision-free. The
`prepend_table_of_contents` convenience API uses this same transaction.
Document language is copied into staging and cannot be changed by the front-
matter callback.

After successful staging, the implementation preflights every stored page and
section index, anchor, footnote reference and field-plan identity. Only then does
it splice the staged pages before the body, insert all front sections at section
zero, merge nonconflicting front anchors/resources, shift all original section
ownership and named destinations, and restore the active cursor to the same body
content position. Callback failure, conflicting anchor names, metadata/outline
mutation or invalid staging ownership leaves the body unchanged.

Successful staged structure roots are inserted before existing body roots, so
top-level semantic reading order follows the physical front-before-body page
order. Their page-local MCIDs and ParentTree ownership are assigned only after
the final splice.

Front footnotes must use section-scoped numbering. Document-scoped front notes
would require renumbering already-painted body markers, so that case refuses
instead of silently creating duplicate or out-of-order numbers.

## Physical-page parity contract

An odd-sized TOC receives one suppressed trailing parity page. Front-matter page
count is therefore always even. Every existing body page keeps its odd/even
physical side, so mirrored margins, odd/even masters and already-positioned page
content remain valid without geometric translation. The parity page belongs to
the front section but suppresses its page master.

Front sections can use their own geometry, masters and Roman/alphabetic page
labels. Deferred TOC values resolve only after the splice, so document-page links
reflect shifted physical pages and section-page links use the shifted original
section owner.

## Unexecuted regression source

Eight source cases cover one-page TOC parity padding with body authority shifts,
an even multi-page TOC without padding, retained footnote reference shifts, and
complete pre-mutation rollback when TOC staging fails. Additional cases cover
arbitrary multi-section blocks with a staged image and anchors, callback-error
isolation, anchor collision and document-scoped front-note refusal. They were
added but not run. A shared structure regression covers front-before-body root
ordering.

## Remaining boundary

This path deliberately preserves body parity instead of repaginating existing
mirrored content. Document-scoped note renumbering, imported-page insertion,
semantic tagging for front-matter tables and notes, complete PDF/UA semantics,
managed bindings and runtime qualification remain open.
