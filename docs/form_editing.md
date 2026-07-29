# Form Editing

The document subsystems form actions resolve an AcroForm field from the canonical,
inherited field-tree report before mutation. Text, choice, checkbox, and radio
updates are type-checked, signature values are rejected by policy, and canonical
form appearance writing produces the saved output. JSON, FDF, and XFDF scalar
data import reuse the form-exchange implementation; selected/full flattening is
performed by the canonical editor.

`form_reset` resolves `/DV` through the same inherited field tree and rewrites
supported text, choice, checkbox, and radio appearances atomically. It skips
signature values under the existing signature policy.

`form_set_default` updates the actual `/DV` dictionary entry for resolved text
and choice fields without changing the live `/V` value. This gives reset a
source-level default rather than a viewer-only convention. Checkbox/radio
default-state authoring remains an exact unsupported boundary because their
named appearance-state and export-value synchronization needs a dedicated
state editor.

`form_set_button_default` supplies that dedicated bounded state editor for a
resolved one-widget checkbox or radio field: it selects an existing named
normal appearance/export state and updates `/DV` only, leaving `/V` and the
live appearance unchanged.

`form_rename` has a source-level path for a resolved non-signature terminal
field: it changes the actual `/T` field dictionary entry through the canonical
object rewriter while preserving the existing widget, value, and appearance
ownership. It accepts one ASCII terminal name only; hierarchy reparenting,
duplicate names, and signature fields are rejected unchanged.

`form_delete` removes a resolved non-signature field subtree from the AcroForm
tree and removes its widget references from page annotations. It never paints
an invisible replacement widget. Unrelated fields and annotations are retained,
and undo restores the exact preimage.

`form_create_text` creates a root text field, widget, page annotation entry,
and canonical normal appearance in one writer transaction. It creates no live
field without an appearance, and requires a unique ASCII root field name and a
finite page-space rectangle.

`form_create_checkbox` uses the same field-tree transaction and replaces the
seed appearance with explicit `Off` and `Yes` named normal states. It
synchronizes `/V`, `/DV`, `/AS`, and `/AP` at creation, so viewers do not need
to invent the checked appearance. Radio-group and push-button authoring remain
separate exact unsupported boundaries.

`form_create_choice` creates either a list box or editable combo box with a
real `/Opt` array, `/V` and `/DV` selection, field flags, widget, and normal
appearance. It currently accepts bounded unique ASCII options and a single
selection; multi-select authoring remains an exact boundary.

`form_set_choice_options` replaces an existing resolved choice field's `/Opt`
array, selected `/V` and `/DV`, and list/combo flags before the canonical form
writer regenerates its appearance. Multi-select option editing remains an
exact boundary.

`form_create_push_button` creates `/Btn` with the push-button flag and normal,
rollover, and down entries in `/AP`. The compact route creates no actions or
host capabilities; callers can attach only separately supported restricted
actions. Caption/icon-layout authoring remains bounded to the existing normal
appearance text path.

`form_create_radio` creates a one-widget radio field with a caller-selected
export name and synchronized `/V`, `/DV`, `/AS`, and named normal state. Adding
or removing choices in an existing multi-widget group remains an exact
topology-sensitive boundary.

`form_move_resize_widget` resolves exactly one widget for a non-signature field
on the requested page and changes its actual annotation geometry through the
canonical source-linked annotation path. Existing valid widget appearances are
preserved or regenerated under that policy; ambiguous multi-widget placement
is refused rather than moving every widget in the field.

Missing fields, incompatible value kinds, unsupported actions, and signature
permission conflicts leave the source unchanged and return typed results.

document subsystems uses the canonical AcroForm hierarchy and form-data editor for
supported FDF/XFDF value changes. Signature policy, validation, inheritance,
and appearance regeneration remain enforced by the existing form runtime.
`form_create_text_in_table_cell`, `form_create_checkbox_in_table_cell`, and
`form_create_choice_in_table_cell` resolve one source-linked table cell and
create normal AcroForm widgets at those exact bounds. They use the canonical
field-tree and appearance writer, and refuse unresolved or invalid cell
geometry rather than painting a table-local substitute.

## Signature fields

`form_create_signature` creates an unsigned `/Sig` field, widget, and blank
canonical appearance through the existing field-tree writer. Existing signature
values remain immutable and any attempt to set or replace them returns
`signature_permission_violation`.

## Calculation order

`form_set_calculation_order` validates each resolved non-signature field and
rewrites the AcroForm `/CO` reference array in the requested order. It does not
execute actions or change field values; cycle detection and restricted action
execution remain owned by the existing action runtime.
