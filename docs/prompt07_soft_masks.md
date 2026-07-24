# Prompt 07 Soft Masks

Soft masks are read from ExtGState `SMask`. `/None` clears the active mask.
Dictionary masks distinguish alpha and luminosity masks, render the `/G` group
into a bounded offscreen surface, and apply the resulting mask to later paint
operations.

## Implemented Common Path

- Alpha masks use the rendered mask surface alpha as coverage.
- Luminosity masks derive coverage from rendered RGB luminance after supported
  DeviceGray, DeviceRGB, and DeviceCMYK paint has passed through the current
  deterministic color converter.
- Mask BBox and Matrix are part of the Form XObject replay posture.
- Text, image XObject, and Form XObject sources paint through the active mask.
- Transfer functions are converted to a lookup table when present.
- Image soft masks with `/Matte` unblend common DeviceGray/RGB/CMYK matte values
  before applying mask alpha.
- ExtGState soft-mask `/BC` backdrop behavior is implemented for common alpha
  and luminosity mask groups.
- Denied mask surface allocation fails closed through the scheduler budget.

## Memory Posture

Soft-mask group surfaces reserve RGBA bytes through the renderer scheduler before
allocation. The Prompt 07 memory budget report uses a 4096 MB audit cap and the
engine default scheduler budget remains 512 MiB unless callers set tighter
decode limits.

The denial test is `renderer_offscreen_surface_fails_closed_over_budget`.

## Prompt 07B Closure

Prompt 07B adds focused fixtures for `image_smask_matte`,
`softmask_alpha_bc_background`, `softmask_luminosity_devicegray`,
`softmask_luminosity_devicergb`, and `softmask_luminosity_devicecmyk`. The
multi-reference audit classifies these as either Wellfriend passing against agreeing
references or Wellfriend inside a reference-disagreement cluster.

Advanced ICC/device-link matte conversion and exact ICC/calibrated luminosity
CMM parity remain unsupported-reported CMM work. Unsupported or malformed mask
structures must be reported instead of silently ignored.

Artifacts:

- `target/prompt07-transparency-compositing/soft-mask-matrix.json`
- `target/prompt07-transparency-compositing/prompt07b-transparency-matrix.json`
- `target/prompt07-transparency-compositing/prompt07b-render-results.json`
- `target/prompt07-transparency-compositing/memory-budget-report.json`
