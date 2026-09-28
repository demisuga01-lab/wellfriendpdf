# Tagged Figure flow - source implementation, not qualification

This extends linked image/caption flow into the existing paragraph ownership
transaction. The complete editor roadmap remains active. No compiler, tests,
PDF workloads, browser execution or rendering were run for this change.

## Request model and authority

`source_tags.figures` maps each retained or explicitly removed figure ID to:

```json
{
  "source": { "object": 42, "generation": 0, "key": null },
  "semantic_text": { "alternate": "Reviewed diagram description", "expansion": null },
  "split_reused_form_semantics": false,
  "outbound_ref_split": null,
  "inbound_ref_split": null,
  "preserve_semantic_subtree": false,
  "delete_semantic_subtree": false,
  "clone_semantic_subtree_for_reused_form": false
}
```

The source is a selected content-only Figure owner in the same approved
contiguous sibling interval as the captions. It is normally a leaf. With
`preserve_semantic_subtree`, the root may also retain a bounded descendant tree
whose children are semantic-only: they must have correct unique parentage and
own no MCID, MCR, OBJR, text or paint. The root's direct source-content position
is replaced by the generated page MCR without flattening or reordering its
children. Valid explicit descendant `/Pg` bindings are rebound to the Figure's
destination page. The approval persists across save/reopen. A removal requires
the separate one-shot `delete_semantic_subtree` decision; the complete validated
tree leaves the live structure only when no surviving element or effective
attribute references any member. For a reused-Form occurrence, preservation also
requires the one-shot `clone_semantic_subtree_for_reused_form` decision. The
selected Figure keeps the original descendants; the residual Figure receives a
copy-on-write clone with rebuilt `/P` and `/K` links. Clone-only `/ID`, `/Pg` and
private story keys are removed so the copy cannot duplicate document identities
or retain the selected page binding. Direct, array or one-level-indirect `/Ref`
links inside the Figure/subtree are rewritten to the corresponding clones.
External links on any subtree member use the binding's subtree-wide outbound
move/retain/copy and incoming follow/retarget/reference-both policies. All
affected outside referrers are rewritten atomically and receipt-checked. When
multiple Figure trees split in one request, a coordinated map keeps original
members linked to originals and residual members linked to peer clones. Its paint
may be page-owned or may be one uniquely invoked nested Form image owned by exact
Form-MCR content. `/RoleMap` aliases to Figure are resolved.
Image and caption never reuse the same semantic element. New ownership for an
untagged image uses `source: null` and requires explicit `semantic_text` review.
For an unchanged source image, omitted/null review preserves existing Alt/E;
an explicit review replaces/removes those values. Captions retain the separate
paragraph-description review rules. This does not infer an image's meaning.

The preflight maps an exact source image occurrence or native capsule to its
enclosing MCID owner. For nested Forms it validates every resource edge, scans
the leaf Form in its effective resource scope, and requires the Figure's complete
content-item set to equal the selected image's MCIDs. A reused Form still refuses
by default because one Form MCR cannot distinguish painted invocations. With the
explicit `split_reused_form_semantics` decision, the existing Figure owner moves
with the selected occurrence while a residual Figure leaf retains the original
Form MCR for unselected invocations. The residual preserves semantic attributes,
gets the approved parent, and drops duplicate `/ID` and private story keys.
When the source has an outbound `/Ref`, the request must additionally choose
`move_with_selected`, `retain_with_residual`, or `copy_to_both`; a policy on a
source without `/Ref` also refuses. The private transaction records both final
values and rejects drift before publication. If other structure elements point
to the source Figure through `/Ref`, `inbound_ref_split` must independently
choose `follow_selected`, `retarget_residual`, or `reference_both`. All affected
referrers are rewritten in one batch and receipt-checked after page insertion.
Bounded direct/indirect array graphs are parsed recursively, retain their nesting
and shared-container topology, and copy-on-write every affected indirect path.
Cyclic, excessive, non-array-indirect or non-structure-leaf containers still
refuse. It checks all
selected paint, not just the presence of a
Figure element: wrong owners, duplicate bindings, unselected artwork, visible
or unselected text inside a Figure, foreign scopes, untransferred content-level
ActualText and nested semantic/OBJR owners are rejected.
The later whole-Figure OCR continuation permits complete explicitly selected
invisible text and direct ActualText when the same Figure owns image and OCR;
see `tagged_ocr_implementation.md` for its independent provenance checks.
One or more exact direct sibling `P`/`Span` owners can instead be consumed under
explicit `merge_into_figure` policies that partition the selected span IDs; see
`separate_ocr_owner_implementation.md`. Owners with independent semantics,
relationships, subtrees or additional content still refuse. The same policy is
source-bound inside Forms; an approved reused-Form split retains adjacent
residual Figure and OCR-owner leaves for the other invocations.
Selected Figure paint may originate on a page outside the text frames; those
source-page streams are also isolated before tag delimiters are rewritten.

## One coordinated transaction

1. Validate text, image and structural ownership on the original revision.
2. Detach all selected image paints in the existing immutable-source batch. A
   nested image uses a bounded leaf-to-page clone tree and retains every shared
   original object.
3. Clone shared streams on text-frame and image-source pages. Nested source
   clones carry a private exact clone-to-source receipt; the tag transaction
   removes only approved Figure MCID delimiters, consumes that receipt, and
   leaves unrelated Form namespaces unchanged. Shared property dictionaries
   are never modified.
4. Rewrite text, paginate and insert continuation pages. Temporary references
   keep semantic owners and native capsules reachable through object remapping.
5. Place native capsules inside Figure provenance markers, separate from their
   byte-verified native image markers. Final placement does not rasterize images.
6. Assign nonconflicting page MCIDs, verify that each Figure marker owns exactly
   its expected image, and update its `/K` to the final page MCR.
7. Order each Figure immediately before its approved caption, update layout
   BBox/Width/Height with per-owner attribute materialization, preserve unrelated
   semantic properties, rebuild ParentTree/IDTree and save rebound story data.
   When an approved reused-Form split exists, insert its residual leaf immediately
   before the moved story interval and shift the saved insertion index so later
   edits still select only the moved Figure and caption owners.

The native image marker hash stays stable when its outer Figure MCID changes.
Stable semantic keys survive canonical object-number remapping. Figure keys
use a separate hash domain from paragraph keys. Existing descriptions belong
to the unchanged image, not to newly generated caption text.

Explicit figure deletion removes its selected current paint and semantic owner
in the same transaction. A surviving owner referencing the removed Figure is
rejected until that relationship is explicitly migrated. Consumed removal
commands and Figure bindings disappear from the saved request. Incremental
historical bytes and unused resources remain possible; this is not redaction.
Structural merge now reports a conflict when caption deletion would orphan a
retained figure, instead of returning that draft as a resolved merge.

The browser editor exposes separate Figure-owner reuse/create selection and
description review plus an explicit reused-Form semantic-split control. It uses
the same preview receipt/checkpoint/undo flow;
there is no new standalone tag bypass. The standalone image mover still refuses
tagged movement, which must use this coordinated story route.

## Regression source and remaining work

Unexecuted regression cases cover:

- Figure growth through canonical page insertion, contraction and repeated save.
- Native image ownership, MCR/ParentTree association, preserved Alt and refreshed
  geometry attributes after reopen.
- Current-paint and semantic-owner deletion with consumed commands.
- Conflicting Figure/caption/image bindings rejected before mutation.
- Alternate-description changes kept separate from caption semantics.
- New Figure creation requiring review for an untagged source image.
- Caption-deletion merge conflicts and referenced-owner deletion refusal.
- Named-property ActualText detection and refusal before image capture.
- Unique nested Form-MCR migration to a generated page MCR, preservation of the
  original Form object, private-marker consumption and repeated saved editing.
- Reused nested Form occurrences refusing until the caller makes an explicit
  semantic split decision.
- Explicit reused-Form splitting preserving one residual Form MCR, moving the
  original Figure owner to one page occurrence, consuming private markers,
  rebuilding ownership and supporting another saved edit.
- Outbound `/Ref` splitting refusing without authority and honoring move,
  retain and copy decisions without carrying the one-shot policy into reopen,
  including an indirect array container.
- Incoming `/Ref` splitting refusing without authority and honoring follow,
  retarget and reference-both decisions across repeated saved editing. Nested
  direct/indirect topology remains nested and affected indirect paths are cloned.
- Shared indirect graphs are rewritten per referrer while repeated containers
  retain aliasing inside each private result; mixed non-structure values,
  excessive depth/items and cyclic indirect containers refuse before mutation.
- Exact same-Figure invisible OCR in a uniquely invoked nested Form, including
  Form-target mismatch refusal and saved reopen/edit; see
  `nested_form_ocr_implementation.md`.
- Explicit page-owned sibling P/Span OCR merge, including multiple exact owners
  for one Figure, consumed one-shot ownership,
  ParentTree/search preservation and refusal of ambiguous or semantic owners;
  see `separate_ocr_owner_implementation.md`.
- Explicit singular and multi-owner OCR in unique and reused Forms, with
  source-bound ordered residual Figure/OCR owners and repeat saved editing.
- Explicit contentless semantic-subtree preservation with nested parent/order,
  metadata, generated-MCR placement, repeat save, descendant page rebinding,
  malformed-page-binding refusal, one-shot complete-tree deletion and external-
  relationship refusal checks.
- Reused nested-Form preservation requiring explicit clone authority, retaining
  the original tree on the selected Figure, creating a residual tree with
  rewritten parent/child and internal relationship links and no copied identities
  or page bindings, plus subtree-wide explicit external relationship decisions.
- Coordinated cross-Figure relationships, retaining selected-to-selected and
  residual-to-residual topology when multiple Figure trees split together.

This is bounded Figure migration, not arbitrary tagged-document editing or
PDF/UA certification. Content-bearing or externally shared reused-Form Figure
subtrees, general visible image+text groups,
relationship values outside the bounded reference/array graph, named/shared ActualText transfer,
mixed image/table cells, namespaces
beyond the supported RoleMap route,
page transparency/rotation migration and broad visual/semantic qualification
remain open. Existing refusal paths continue to protect those cases.

Generated tagged wrappers can retain empty graphics-state scaffolding across
checkpoints; broader ownership-aware compaction and performance qualification
remain separate. No output-fidelity or performance result is claimed.
