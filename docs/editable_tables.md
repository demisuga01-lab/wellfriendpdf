# Editable Tables

Prompt 34 projects `analysis::tables::Table` into a deterministic,
source-linked `EditableTableGraph`. Ruled, borderless, partial-rule, and
semantic-table evidence stays separate from the editable cell identity. A cell
edit re-resolves the current table and its origin cell, then invokes Prompt 33
source rewriting inside that cell's PDF-space bounds. It does not cover the
cell with replacement artwork.

`table_edit_cell` rewrites existing cell text through the same geometric or
semantic reflow policy as other text. `table_append_row` and
`table_append_column` have executable conservative structural paths for an
unmerged ruled grid: they write new vector borders and text instructions only
in verified empty space below or beside the table. They reject page images,
annotations, source text, merged cells, ambiguous grids, oversized values, and
missing page capacity rather than moving unknown neighbors or covering content.

`table_set_cell_alignment` retains the resolved cell text and uses Prompt 33
to rewrite its actual source positioning under a bounded left/right/center/
justify/start/end policy. It does not draw a replacement text layer.

`table_set_cell_padding` similarly retains the resolved cell source text, but
reduces its usable PDF-space reflow bounds by explicit left, bottom, right,
and top insets.  It rewrites actual text positioning only when the resulting
region remains usable; negative, non-finite, or over-constraining padding is
an exact no-change refusal.

`table_add_cell_border` appends a canonical stroked PDF rectangle at one
resolved cell boundary. It changes actual page-content instructions while
leaving the cell text untouched; it rejects invalid widths and unresolved cell
geometry rather than adding a painted cover-up.

`table_set_cell_fill` appends a canonical filled rectangle as an underlay, so
the original cell text and supported borders remain above it. Its RGB and
opacity inputs are bounded; invalid values or unresolved cell geometry refuse
without writing a replacement layer.

`table_edit_math_cell` is the explicit cross-system path for a born-digital
mathematical source already resolved inside a table cell. It requires review
approval, retains the table-cell provenance, and uses the same Prompt 32
shaping and Prompt 33 cell reflow as a standalone supported math edit.

`table_move_linked_annotation` resolves the current cell bounds and moves only
the named source-linked annotation through the canonical Prompt 17 geometry and
appearance path. The table source is not repainted or rebuilt; unknown cells
and invalid bounds are exact refusals.

`form_create_text_in_table_cell`, `form_create_checkbox_in_table_cell`, and
`form_create_choice_in_table_cell` are the reciprocal integration paths: they
create real AcroForm field/widgets at an explicitly resolved cell's bounds
while leaving the table's content and borders intact.

Insertion at an interior position, deletion/reordering, column topology,
merged-cell surgery, and multi-page continuation remain exact no-change
boundaries until their source paths, downstream dependencies, and page-flow
constraints can be proven. Table analysis retains span, header, nested-table,
row/column, bounds, confidence, and provenance evidence for those operations.

Table evidence combines canonical table analysis, Prompt 33 semantic regions,
and provenance-resolved cell text. Supported cell edits use real source
rewriting and bounded reflow; ambiguous grids and unsupported pagination
return typed no-change results.
