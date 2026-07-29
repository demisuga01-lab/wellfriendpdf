# Mathematical Content Editing

Born-digital formula-like text is parsed into a source-linked expression tree
covering rows, fractions, radicals, scripts, fenced expressions, identifiers,
operators, numbers, and unknown leaves. Replacement is deliberately approval
gated and is written through Prompt 32/33 shaping and font reconstruction.

The deterministic bracket-matrix form `[[a,b];[c,d]]` is also executable:
`math_edit_matrix_cell` changes exactly one resolved cell, rebuilds the source
expression, and passes the result through the same shaped source-reflow and
undo path. Other matrix notations remain `math_structure_not_resolved`; they
are never flattened or approximated as ordinary prose.

`table_edit_math_cell` is the explicit cross-system route for an approved
born-digital math source that is already provenance-resolved inside a table
cell. It delegates source positioning to the same Prompt 32 shaping and
Prompt 33 cell reflow path as other supported mathematical edits.

`math_move_resize` retains a resolved born-digital source expression and
rewrites its actual Prompt 33 region bounds after explicit approval. It is a
bounded source-geometry operation, not a flattened equation image or overlay.

`math_edit_fraction_part` changes exactly the numerator or denominator of one
resolved single-slash born-digital fraction. It validates the source fragment,
rebuilds the complete fraction, and uses the same approval, shaping, source
rewrite, and undo path.

`math_edit_matrix_structure` inserts or deletes one row or column of a
resolved bracket matrix with exact rectangularity checks. It refuses notation
outside that source form rather than flattening it to ordinary text.

`math_edit_script` changes one resolved source superscript or subscript while
retaining its base expression. Nested script construction remains a typed
boundary until a full OpenType MATH layout route is selected.

The parser does not claim that an outlined or raster formula is ordinary text.
Those cases retain the original visual object, report `formula_review_required`,
and cannot be destructively replaced through this route. Advanced OpenType MATH
construction remains an exact `math_metrics_unavailable` boundary whenever the
resolved source font does not expose usable canonical metrics.

Supported mathematical text changes use Prompt 32 shaping/subsets and Prompt
33 source rewriting. Unresolved outlined or raster formulas retain the original
visual source and require explicit approval before replacement.

## Fenced expressions

`math_edit_fenced_inner` rewrites the inner source of a resolved single-layer
parenthesized, bracketed, or braced born-digital expression while retaining its
delimiter pair. Matrix syntax and unresolved/outlined formulae remain exact
review or unsupported boundaries.

`math_edit_radicand` supports resolved `sqrt(...)` and Unicode radical source
notation. The operation preserves the notation, rewrites only the radicand
through Prompt 33, and refuses malformed or multiline source fragments.
