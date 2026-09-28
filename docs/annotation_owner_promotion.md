# Owner-aware direct annotation promotion

Source implementation only. No compiler, test, PDF workload or renderer has
executed this increment. The complete roadmap remains active and incomplete.

## What the writer now does

The shared promoter no longer rejects every direct Widget or StructParent
annotation. `annotation_promotion_owners.rs` binds the selected page occurrence
to its existing form/structure ownership before allocating or writing:

- Widgets are matched to one exact dictionary reachable through AcroForm Fields
  and field Kids, with checked Parent relationships. Merged root widgets and
  widgets below indirect field parents are supported.
- StructParent identifies the declared ParentTree owner. The selected annotation
  must match exactly one OBJR carrier directly owned by that StructElem.
- If those owners already refer to an equivalent indirect annotation, its object
  reference is reused. Field and tag owners must agree on that reference. An
  object already used by another page annotation is not adopted.
- If the authoritative field or OBJR slot itself is direct, it is replaced with
  the same new annotation reference installed into the selected page slot.
- Multiple changes to one owner compose in an object update map. Arrays below
  that owner are copied on write; an unrelated alias of a shared Kids array is
  not silently modified. Existing indirect dictionary owners retain identity.
- Native field values/defaults, actions, appearance references, semantic roles,
  alternate text and opaque fields remain. A selected indirect integer
  StructParent is normalized to its equivalent scalar value for the canonical
  ownership index. Subtype resolution is now consistent for direct and indirect
  names in shared relationships, native discovery and XFDF export.

The field tree must contain exactly one final reference to each promoted widget.
Tagged promotion invokes the existing complete ParentTree ownership validator
on the unpublished candidate. Dictionary, annotation-order and relationship
postconditions from `annotation_source_promotion.md` still apply. No intermediate
PDF is returned if a later ownership check fails.

Public APIs and the generic document-subsystems action are unchanged. The same
owner resolver now serves automatic promotion in native geometry, XFDF and story
staging as well as explicit source promotion. Promotion reports add
`reused_owner_ids`, `field_ownership_verified` and `tagged_ownership_verified`.
The booleans refer to selected owner classes actually checked; false also means
that no selected source of that class needed promotion.

This follows the separation between widget field ownership and object-reference
structural ownership in Adobe's [PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf),
sections 8.6.2 and 10.6.3. Unique exact matching is the supported normalization
policy, not a claim to infer intent from arbitrary damaged documents.

## Remaining boundaries

Equal direct page occurrences claiming one owner, multiple field candidates,
conflicting field/tag shadow objects, missing owners, malformed or cyclic trees
still need explicit repair/materialization decisions. Unambiguous direct
structure owners/root dictionaries and exact lookup copies are now handled by
the subsequent `annotation_structure_materialization.md` increment. It preserves
lookup keys and roles rather than broadly rebuilding owner indexes.
Direct nonterminal field ancestors with unambiguous tree ownership are now
handled by the subsequent `annotation_field_materialization.md` increment,
including necessary sibling Parent repairs. Conflicts are not silently merged.
Shared tagged appearance cloning and arbitrary semantic subtree changes are
still separate work. Unrelated invalid tagged content can fail the global
ParentTree check. No standards/accessibility conformance certification is implied.

The earlier direct-source page-pruning and appearance-generator boundaries still
apply. This increment is neither sanitizing redaction nor a signature-preserving
or encrypted-writer guarantee.

## Unexecuted regression source

Nine additional tests in `annotation_promotion_owner_tests.rs` cover merged
widgets, direct owner-slot rewriting, field/tag reference reuse, direct OBJR
repair, mixed direct/indirect ownership, native cross-page tagged/widget movement,
conflicting/already-page-owned objects, two edits under one field parent, shared
Kids copy-on-write, stale owner revisions, indistinguishable page copies, and
indirect Subtype/StructParent values. The prior missing-owner regression remains
a refusal test, not an unsupported-all-widgets test.

Only rustfmt formatting/parser checks and whitespace checks were used. These do
not establish Rust type correctness, test success, field interaction, accessible
reading order, rendering fidelity or interoperability.
