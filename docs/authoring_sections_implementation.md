# Fresh-authoring sections - source implementation, not qualification

This increment adds typed section ownership to `FlowDocument`. It changes source
only. No compiler, test, PDF workload, renderer, benchmark, browser, deployment,
commit or push was run.

## Implemented contract

Each flow page now records one monotone section owner. A `FlowSection` owns its
page size, margins, optional physical even-page left/right mirroring, optional
first-page master, odd/even masters, page-number
start and number style, and a section/document-scoped automatic footnote
numbering policy. A later section can start on the next physical page or
on the next requested odd/even page. Intervening parity blanks remain part of
the preceding section and suppress its running master so a recto start cannot
acquire accidental header/footer content.

Running headers and footers consist of literal fragments plus typed fields:

- physical document page and total page count;
- current section label number;
- physical pages in the section;
- final section label number.

Decimal, Roman and alphabetic numbering are supported. Non-default section
sequences emit a catalog `/PageLabels` number tree using `/S` and `/St`. A
single default decimal section omits the redundant number tree to preserve the
older output shape.

Final counts are unavailable during incremental layout, so masters are painted
only on a private builder clone immediately before font/image planning and the
existing canonical serialization path. Each running item is one artifact-wrapped
text line. It must fit within the section content width and wholly within the
reserved top or bottom margin. Failure, cancellation, unsupported Roman values
and geometry drift return an error without altering the caller's builder.

## Invariants

- Every declared section owns at least one page.
- Section indexes are monotone and cannot be interrupted by an unowned page.
- A page's stored size and margins must equal its owning section.
- Mirrored margins swap only left/right on even physical pages and are applied
  by the single page-allocation path used by body, table and note continuations.
- Master materialization is private and idempotent.
- A failed section transition validates before adding parity pages or switching
  the current section.
- Physical odd/even selection is based on the one-based PDF page side, while
  displayed section numbering can restart independently.
- `SectionPages` is a count; `SectionLastPage` is the final displayed number.
- Section-scoped automatic footnotes restart from the section policy; document
  scope retains its monotone counter across section transitions.

## Unexecuted regression source

The source specifications cover first/odd/even selection, artifact boundaries,
final page fields, parity ownership and suppression, mixed page geometry,
Roman PageLabels, serialization/reopen inspection, nonmutation on failure,
idempotence, transition rollback, default-label omission, post-layout geometry
drift, unrepresentable Roman values and physical-page mirrored margins. They were
added but not executed.

## Remaining boundary

This is a fresh-document section model, not automatic recovery of sections from
an existing PDF. Typed fresh-document footnote/endnote layout is implemented in
`authoring_notes_implementation.md`, including exact marker-to-note structure
relationships. Deferred document/section/anchor page fields and named PDF destinations
are implemented in `authoring_fields_implementation.md`. The section layer does
not yet provide list continuation authorities, section-aware authored
table row commands, different first-page body margins, dynamic chapter-title
extraction, accessibility structure for running
content, or linked-story section repagination. Compilation, native/binding
execution, save/reopen validation, independent rendering and corpus qualification
remain pending the VPS gate. It does not establish universal editing or a
better-than-Acrobat claim.
