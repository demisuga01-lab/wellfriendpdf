# Separate tagged OCR owner merge - source implementation, not qualification

This increment extends the linked-story Figure transaction for one bounded PDF
tagging shape: an image is owned by a selected `Figure` leaf while exact
invisible OCR operands may be distributed across explicitly selected, direct
sibling `P` or `Span` leaves. The content can be page-owned or carried by a
unique or explicitly split reused Form XObject. No compilation, test, PDF,
renderer, browser or binding workload was run.

## Explicit ownership contract

The caller must select the complete contiguous sibling interval and set
`source_tags.figures[figure_id].separate_ocr_owners` to the exact source elements
with policy `merge_into_figure`. Every entry carries a nonempty, disjoint subset
of the Figure's exact `ocr.span_ids`. Several spans may name the same owner in
one entry, but one owner cannot appear in several entries or figures. The legacy
singular `separate_ocr_owner` remains accepted; an omitted `span_ids` list on
that spelling means all selected spans. The singular and plural spellings cannot
be combined. The engine does not infer a semantic owner from matching text,
geometry, MCIDs or proximity.

Preflight requires that the named source:

- is a unique selected direct child of the approved story parent;
- is distinct from every paragraph and Figure owner;
- resolves, including bounded RoleMap aliases, to `P` or `Span`;
- is a content-only leaf and owns the exact selected `Tr 3` operands in the
  page or Form content namespace;
- contains only the structural keys `Type`, `S`, `P`, `K`, `Pg` and `ID`;
- is not repeated, shared between two figures or retained by another binding;
- owns exactly the named selected spans, while unassigned spans must remain
  owned by the Figure itself; and
- contains no unselected text, image, vector paint, nested structure or OBJR.

An owner carrying `Alt`, `E`, `Lang`, `A`, `C`, `Ref`, namespaces or another
semantic/relationship property refuses. Moving those values to a Figure would
change their meaning, so a future policy must describe each case explicitly.

## Atomic result

Original-revision validation binds each OCR span to its approved separate owner
or, when no separate assignment exists, to the Figure owner.
The existing capture transaction removes the exact image and search operands,
preserves text advances, and creates one native visual/search group. Tagged
preparation removes both source marked-content wrappers. For a nested occurrence,
the existing bounded leaf-to-page clone transaction isolates the exact visual
and search programs while retaining the shared source objects. When a Form has
other invocations, the existing explicit semantic-split decision now clones the
Figure and every selected content-only OCR owner, retains their original MCRs as
ordered adjacent residual siblings, strips IDs/private story keys and validates
every receipt again after the mutation. The selected sibling interval is
replaced atomically: each empty `P`/`Span` owner disappears,
while the moved Figure's final page MCR owns the complete native group.
ParentTree and saved story bindings are rebuilt on the reopened output.

The one-shot singular and plural ownership decisions are cleared from the saved request.
Subsequent save/reopen/edit and whole-group deletion operate on the native group
without repeating or retaining stale source ownership instructions.

## Unexecuted regression source

Added source regressions cover direct and split content streams, ParentTree and
search-text preservation, removal of source `Span` owners, consumed one-shot
decisions, two exact owners merged into one Figure, and repeat save/reopen.
Refusal cases cover omitted approval, a paragraph or nonselected owner, singular
plus plural syntax, empty/stale/overlapping span partitions, repeated owners,
one owner assigned to two figures, semantic attributes and additional
selected-owner text. Further source regressions cover unique and reused Form
MCRs with singular and plural owner sets, including residual owner ordering and
repeat saved editing. These tests have not been run.

## Deliberate boundaries

This is not general subtree flattening. Owners with meaningful attributes or
relationships, nested P/Span subtrees, optional-content OCR and partial/shared
ActualText remain explicit refusals. Image inpainting, arbitrary scan
orientation, handwriting, font reconstruction, PDF/UA conformance and corpus
fidelity are separate roadmap and qualification work.
