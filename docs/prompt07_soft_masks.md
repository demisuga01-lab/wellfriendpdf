# Prompt 07 Soft Masks

Soft masks are read from ExtGState `SMask`. `/None` clears the active mask.
Dictionary masks distinguish alpha and luminosity masks, render the `/G` group
into a bounded offscreen surface, and apply the resulting mask to later paint
operations.

## Implemented Common Path

- Alpha masks use the rendered mask surface alpha as coverage.
- Luminosity masks derive coverage from rendered RGB luminance.
- Mask BBox and Matrix are part of the Form XObject replay posture.
- Text, image XObject, and Form XObject sources paint through the active mask.
- Transfer functions are converted to a lookup table when present.
- Denied mask surface allocation fails closed through the scheduler budget.

## Memory Posture

Soft-mask group surfaces reserve RGBA bytes through the renderer scheduler before
allocation. The Prompt 07 memory budget report uses a 4096 MB audit cap and the
engine default scheduler budget remains 512 MiB unless callers set tighter
decode limits.

The denial test is `renderer_offscreen_surface_fails_closed_over_budget`.

## Partial Behavior

The implementation does not claim exact matte/background parity for every PDF
edge case. Luminosity masks use the current device-space color posture; exact
ICC-managed luminosity conversion remains partial. Unsupported or malformed mask
structures must be reported instead of silently ignored.

Artifacts:

- `target/prompt07-transparency-compositing/soft-mask-matrix.json`
- `target/prompt07-transparency-compositing/memory-budget-report.json`
