# Transparency Rendering Knockout and Isolation

Transparency Rendering implements the common transparency group path for isolated groups,
non-isolated groups, nested groups, and knockout flags. These behaviors are
exercised by focused corpus fixtures and reported against Poppler, PDFium, and
MuPDF.

## Implemented

- `/I true` isolated groups start from a transparent surface.
- `/I false` non-isolated groups copy the parent backdrop, replay content, and
  subtract the copied backdrop before compositing back to the parent.
- `/K true` knockout groups retain the group's initial backdrop in the
  offscreen buffer. Each covered interior pixel recomposes against that initial
  backdrop, so later overlapping objects knock out earlier group objects.
- Nested isolated and knockout groups recurse through the same scheduler-admitted
  group surface path.
- State restore covers alpha, blend mode, CTM, clip, and soft-mask state through
  ordinary graphics-state save/restore.

## Evidence

The Transparency Rendering corpus includes isolated, non-isolated, knockout, nested isolated,
nested knockout, clipped group, and Form XObject group fixtures. Results are
written to:

- `target/transparency_rendering-transparency-compositing/group-isolation-knockout-matrix.json`
- `target/transparency_rendering-transparency-compositing/post-implementation-render-results.json`
- `target/transparency_rendering-transparency-compositing/transparency_closeout-render-results.json`
- `target/transparency_rendering-transparency-compositing/transparency_closeout-transparency-matrix.json`

## Transparency Closeout Closure

Transparency Closeout adds `knockout_overlap_exact` and
`knockout_overlap_nested_form`. Poppler, PDFium, and MuPDF disagree on these
semi-transparent overlap fixtures; Wellfriend matches MuPDF and is classified inside
the reference cluster, with zero Wellfriend-outlier failures.

Remaining bounds are later-owned: text clipping inside knockout groups and
pattern/shading paints inside knockout groups are not Transparency Closeout work.
