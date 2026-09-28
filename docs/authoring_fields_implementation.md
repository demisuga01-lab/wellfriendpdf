# Fresh-authoring body fields - source implementation, not qualification

This increment adds named destinations and forward-capable page fields to fresh
flow authoring. It changes source only. No compiler, test, PDF workload,
renderer, benchmark, browser, deployment, commit or push was run.

## Named anchor contract

`FlowDocument::add_anchor` binds a bounded unique name to the current physical
page, section and vertical cursor. Final authoring validates that ownership and
emits the anchors in a sorted catalog `/Names` `/Dests` name tree as XYZ page
destinations. Final serialization builds a bounded 64-way indirect name tree:
every leaf and branch publishes exact `/Limits`, keys are sorted by final encoded
PDF text-string bytes (rather than source-language Unicode ordering), and
encoding collisions fail closed. This avoids one linear 100,000-entry catalog
dictionary while retaining the public anchor budget. Duplicate, empty, multiline
and invalid retained anchors fail closed. Because fresh flow pages append rather
than insert before completed pages, the stored page identity remains stable.

## Deferred field contract

`add_field_paragraph` accepts literal parts and typed fields for:

- current physical document page and final document page count;
- current section page label, section page count and final section label;
- a named anchor's physical or section-labelled page.

Forward anchor references are legal during layout. Each field declares a maximum
character capacity and optional fixed prefix/suffix. Layout uses an all-eight
placeholder at that capacity, records exact line ranges and positions, and adds
private deferred commands in the original painting order. Form feeds retain the
same physical-page semantics as normal flow paragraphs.

Resolved values may align left, center or right inside that character-capacity
box. Padding remains painting-only while the exact raw-value range drives link
geometry and logical text omits all padding.

After note and section materialization establish final page ownership, the field
stage resolves every plan. It collects the exact page of every line, applies the
target section's decimal/Roman/alphabetic authority, pads the painted value
inside the declared capacity, shapes the original line ranges with whole-
paragraph bidi and fallback context, and preserves unpadded final text through
the logical-text owner. Center/right alignment is recomputed inside the reserved
line width. The existing font planner and canonical writer then consume only
ordinary resolved commands.

An unresolved anchor, value wider than its declared character capacity, final
line wider/taller than its placeholder reservation, duplicate/missing plan line,
or damaged source identity rejects the output. No final field can silently choose
new line breaks or repaginate later content. Materialization is private and
idempotent; failed paragraph layout restores commands, pages, cursor and plan ID.

## Optional native link contract

An anchor-page field can request `BodyFieldFormat::link_to_anchor(true)`.
Non-anchor fields reject that option before layout mutation. During final field
resolution the implementation recovers the anchor value's horizontal advance
interval from the same paragraph-level shaping result used for painting,
including bidi ordering and multi-font fallback runs. It combines that interval
with the resolved baseline ascent/descent, validates a finite nonempty rectangle
inside the owning page and publishes one `/Subtype /Link` annotation whose
`/Dest` string names the catalog XYZ destination. The annotation has a zero
border, print flag, deterministic plan/field-bound `/NM`, explicit owning-page
`/P` reference and bounded human-readable `/Contents`. Duplicate or
overlapping generated hitboxes reject the private materialization transaction.

Annotation objects receive independent indirect identities and each owning page
references them through `/Annots`. Re-materializing an already resolved clone is
idempotent and does not append another annotation. This is a navigation
contract, not semantic PDF/UA link tagging.

## Unexecuted regression source

Eighteen source cases cover forward references, exact named-destination page
objects, unresolved-target failure, final-count capacity overflow, target section
Roman labels, distinct document/section counts, oversized-placeholder rollback,
duplicate/invalid anchors, link validation and rollback, exact finite hitboxes,
mixed-direction geometry, distinct same-line links, annotation dictionaries,
page `/Annots` ownership, encoded name-tree ordering/limits, bounded multi-leaf
tree construction, fixed-capacity value alignment and materialization idempotence. They were added but not
executed.

## Remaining boundary

The fields are page-value fields, not a general expression or indexing engine.
Generated link annotations are not yet integrated into an accessibility
structure tree. Deferred fields currently use one paragraph style; rich inline
runs, tab-stop-exact field
boxes, table-of-contents generation, figure/table/equation numbering, dynamic
chapter text, semantic cross-reference tags and imported/linked-story anchors
remain open. Executable build, save/reopen, navigation, extraction, independent
rendering and corpus qualification remain pending the VPS gate. This does not
establish universal editing or a better-than-Acrobat claim.
