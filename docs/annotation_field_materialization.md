# Direct field-ancestor materialization

Source implementation only; not compiled or runtime-qualified. The complete
editor/rendering roadmap remains active. No tests or PDF workloads were run.

## Canonical transaction

`annotation_field_materialization.rs` extends the shared owner resolver rather
than introducing a separate form writer. It indexes Fields/Kids paths, field
object identities, exact widget dictionaries and declared Parent relationships.
Cycles, shared field objects, conflicting explicit parents and ambiguous page
widget copies remain errors. A missing Parent immediately below a direct field
is reconstructed from that exact field-tree position.

Before mutation, the planner finds direct field ancestors of selected widgets
and expands the necessary child dependencies. Materializing a direct field
requires updating its immediate children's Parent pointers. Expansion continues
through direct nonterminal children but stops at stable indirect subtrees unless
another selected path requires further work there.

Affected page widgets are bound to their exact source occurrences and included
in identity staging before revision hashes change. One selected widget can
therefore require normalizing siblings' ownership, but does not authorize moving
or resizing them. Hidden non-widget terminal fields retain their values and are
materialized only when their parent changes. Existing indirect descendants,
appearance references, values/defaults, opaque fields and child ordering remain.

The writer allocates all needed references before rewriting parents. Child
updates compose bottom-up, and changed boundary slots are installed through the
same copy-on-write container updater used by source promotion. Unchanged Kids
references remain references; changed shared arrays are copied into their
actual owner rather than modified globally. Original ancestor dictionaries and
slots are checked against the current staged revision before rewriting.

The same planner also binds a direct merged field copy to an already indirect
page widget. Thus automatic normalization is not gated solely on whether the
page's Annots entries are direct.

Native geometry, XFDF and story identity staging all use the shared promotion
path. Final field ownership, dictionary, annotation-order and relationship
checks still run in the implementation. Tagged dependencies also invoke the
existing ParentTree validator on the unpublished candidate. No partial PDF is
returned if a later ownership check fails.

## Receipts and invalidation

Promotion reports add:

- `materialized_field_nodes`: newly materialized non-widget field nodes,
  including necessary ancestors and hidden terminal-field dependencies;
- `repaired_field_parents`: Parent values changed by the field normalization;
- `dependent_widget_ids`: page widgets whose field ownership required staging,
  which can include the originally selected widget.

Native batch and XFDF relationship reports carry one optional
`source_normalization` receipt. The single-geometry wrapper transfers that
receipt to its single result; batch rows do not each clone a potentially large
dependency receipt. Its output hash describes the internal normalization
revision, not the final geometry/XFDF output. Outer input/output hashes remain
authoritative for the complete operation.

Native geometry invalidation includes dependent-widget pages even if those
widgets did not move. The single operation exposes `affected_pages`, which the
generic document-subsystems route now uses instead of only source/destination.

## Boundaries still open

Direct structure owners, hidden direct tagged widgets without page-bound owner
provenance, and structural ownership on direct non-widget fields still require
separate materialization decisions. Contradictory parents, competing field/tag
objects, indistinguishable copies and arbitrary malformed-tree repair are not
silently resolved. This does not infer missing field names, types, formulas or
user intent, certify field interaction/accessibility, remove revision history,
or guarantee signature/encryption preservation.

Direct-source continuation pruning, automatic direct-source appearance generation,
shared tagged appearance cloning and the broader roadmap also remain open.

## Source regression coverage and checks

Eight unexecuted tests in `annotation_field_materialization_tests.rs` cover
nested direct ancestors, hidden field values, sibling identity/geometry
preservation, stable indirect subtrees, repeated normalization, indirect page
widgets, direct merged field copies, contradictory/ambiguous ownership, stale
ancestor revisions, dependency-page invalidation and combined field/tag owners.
Assertions inspect actual resulting objects when these tests eventually run;
none of those outcomes is presently runtime evidence.

Only rustfmt formatting/parser checks and whitespace checks were performed.
Those do not establish Rust type correctness, runtime correctness or product
readiness. The full work remains tracked in `universal_editor_roadmap_tracking.md`.
