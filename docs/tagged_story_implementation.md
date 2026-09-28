# Tagged linked stories — bounded source implementation

Date: 2026-09-16. Dirty candidate over main
`27e62db3a1b84804339e65b6025273fd003b3736`.

No compiler, build, type check, tests, PDF workload, renderer, benchmark or
deployment ran. Rust formatting/parser checks, JavaScript syntax checks and
tracked diff whitespace checks are not executable qualification.

Subsequent table work uses a separate complete Table/TR/TH/TD ownership model
inside this shared transaction, not a relaxation of paragraph-leaf selection.
See [table pagination and tagging](paginated_table_implementation.md) for
rowspan fragmentation, header associations, span attributes and explicit limits.

## What is connected

`engine::tagged_structure::story` now participates in linked-story validation,
preview, checkpoint and saved-metadata rebinding. The prior blanket rejection of
every tagged input has been replaced by an explicit, revision-bound selection:

- `parent` and `selected` identify a contiguous sibling interval in the existing
  structure tree. The caller approves its reading order and complete text scope.
- `paragraph_sources` maps paragraph IDs to a reused selected owner, or `null`
  for a new owner. Ordinal defaults are allowed only when counts match. Reused
  roles/IDs and unrelated siblings remain; new paragraph roles default to `P`.
- `new_roles` is an explicit role choice for newly created owners, not inferred
  semantics or proof of role/parent compatibility.
- `semantic_text` explicitly replaces/removes existing `Alt` and `E` wording.
  The source inventory exposes their current values. Existing descriptions cannot
  silently survive a changed paragraph without a decision. After checkpoint,
  remaining descriptions need review again on the next edit.

The supported source unit is a complete page-owned paragraph leaf, not an
arbitrary tagged subtree. Preflight checks the actual content operators against
selected source ranges/owned frames. Partial source operands, selected tags with
unselected text, clipping text, non-text paint inside selected tags, optional
content, nested semantic children, OBJR owners and Form-owned content require a
different migration. `text_leaf` in the inventory is only a structural candidate;
the mutation preflight performs the stronger content check.

## Private transaction

1. Validate the original source revision, frame hashes, structure ownership and
   explicit sibling selection. Check surviving structural references, including
   indirect `/Ref` arrays and table-header ID relationships to removed owners.
2. Clone shared page-content occurrences. Detach only approved old BDC/EMC
   delimiters, including wrappers crossing content-stream boundaries. Preserve
   enclosed operators and shared named property resources. This prevents stale
   named ActualText from interfering with the glyph deletion postcondition.
3. Rebind the private prepared frame hashes once, before any frame rewrite. The
   ordinary source editor replaces glyph content and the existing paginator
   creates continuation pages in its canonical insertion batch.
4. Generated lines carry paragraph and frame identity markers. After object/page
   renumbering, assign page-local MCIDs, verify exact planned line ownership/order,
   and build MCR entries with final page references. `/K` uses approved story
   order, not accidental paint-stream order. Update paragraph ActualText and the
   explicitly reviewed alternate/expanded text.
5. Replace only the approved sibling interval, remove the private transaction
   marker and rebuild/validate ParentTree and IDTree against actual output.
   Rebind frame hashes and persist the resulting paragraph identities.

Persistent paragraph/parent keys survive canonical object renumbering. The
temporary transaction uses real PDF references, not object numbers inside JSON,
so canonical rewriting can remap them. Intermediate candidates never leave the
transaction. Failures/cancellation withhold output; this remains ordinary editing,
not sanitizing redaction of incremental history.

## APIs and browser

- Universal analysis: opt in with `include_story_tag_sources: true`. The result
  supplies the source SHA and inventory, or an explicit ownership-repair boundary.
  Ordinary lazy page analysis does not automatically perform the full tag scan.
- Universal linked-story plans include the selected tag identities in the
  candidate, read/write sets and approval reasons.
- WASM session: `tagSourcesJson()`. Worker client: `tags()`.
- The browser editor can load/select sibling owners, choose per-paragraph reuse
  or creation, and review alternate/expanded text before preview/checkpoint.
- The structural merger materializes base ordinal ownership before paragraph
  insertion/deletion/reordering. Source selection and mutation authority remain
  fixed; owner retargeting and non-default new roles require explicit replanning.

The same final preview/checkpoint approval still binds the full request hash.
This does not add authenticated authorization or a production application rollout.

## Regression source — not executed

New source cases cover tagged growth into inserted pages, contraction, save/reopen,
empty/refilled stories, canonical renumbering, unchanged sibling roles, invalid
partial selections, shared named ActualText and cross-stream delimiters, dangling
direct/indirect semantic references and header IDs, and alternate-text review
renewal. A merge case asserts that ownership follows paragraph identity through
reordering, insertion and deletion. No passing result is claimed.

## Still open

This is not general PDF/UA-preserving repagination. Inline semantic subtrees,
lists/tables/notes, Form occurrences, arbitrary role maps/namespaces/extension
relations, automatic semantic inference, complete layout-attribute migration and
meaning-dependent accessibility validation remain work. The structural ownership
checker is not a conformance certificate. Original headers, artwork, tags and
semantics outside the approved ownership scope are not inferred or repaginated.

The full editor/rendering roadmap remains active in
`universal_editor_roadmap_tracking.md`. Current builds, bindings, regression runs,
independent extraction/rendering and real-document accessibility checks are still
required on the VPS. Neither universal editing nor superiority to Acrobat has
been established.

## Primary references

- [Adobe PDF Reference 1.6](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf): structure ownership, MCRs, ParentTree and replacement text.
- [PDF Association specification corrections, document interchange](https://pdf-issues.pdfa.org/32000-2-2020/clause14.html): structural relationships, namespaces and replacement-text semantics.

These sources informed the ownership model; they are not evidence that this
implementation passes the specifications.
