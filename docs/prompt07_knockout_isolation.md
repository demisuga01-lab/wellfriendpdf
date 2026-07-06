# Prompt 07 Knockout and Isolation

Prompt 07 implements the common transparency group path for isolated groups,
non-isolated groups, nested groups, and knockout flags. These behaviors are
exercised by focused corpus fixtures and reported against Poppler, PDFium, and
MuPDF.

## Implemented

- `/I true` isolated groups start from a transparent surface.
- `/I false` non-isolated groups copy the parent backdrop, replay content, and
  subtract the copied backdrop before compositing back to the parent.
- `/K true` knockout groups are detected, counted, and use the current native
  knockout approximation in the render buffer.
- Nested isolated and knockout groups recurse through the same scheduler-admitted
  group surface path.
- State restore covers alpha, blend mode, CTM, clip, and soft-mask state through
  ordinary graphics-state save/restore.

## Evidence

The Prompt 07 corpus includes isolated, non-isolated, knockout, nested isolated,
nested knockout, clipped group, and Form XObject group fixtures. Results are
written to:

- `target/prompt07-transparency-compositing/group-isolation-knockout-matrix.json`
- `target/prompt07-transparency-compositing/post-implementation-render-results.json`

## Remaining Bound

Exact PDF knockout behavior for every interior object-overlap case remains a
measured partial implementation. The public feature report says this explicitly
as `exact_pdf_knockout_interior_overlap`.
