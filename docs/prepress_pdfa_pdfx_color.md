# Prepress, PDF/A, and PDF/X Color Scope

Prompt 05 adds color-specific reporting and validation hooks. It does not claim
complete PDF/A or PDF/X conformance.

## Output Intents

`ColorReport` scans the catalog `/OutputIntents` array and reports:

- `/S`
- `/OutputConditionIdentifier`
- `/OutputCondition`
- `/RegistryName`
- `DestOutputProfile` presence
- output profile `/N`
- decoded profile byte length
- qcms basic ICC parse success when below cap

Validation profiles:

- `Generic`: inventory only.
- `PdfA`: emits `color.output_intent.missing` if no output intent exists and
  `color.output_intent.profile_missing` if an output intent lacks
  `DestOutputProfile`. Prompt 05B also diagnoses output-intent `/S` values that
  are not `GTS_PDFA1`.
- `PdfX`: same required output-intent/profile checks as `PdfA`, plus Prompt 05B
  diagnoses output-intent `/S` values that are not `GTS_PDFX` and warns when a
  parseable destination profile exposes `/N` other than 4. Broader PDF/X rules
  remain outside this color-only subset.

## Spot Colors and DeviceN

Separation and DeviceN are preserved in report metadata:

- spot colorant names are listed under `spot_colorants`;
- DeviceN component arrays are listed under `devicen_components`;
- DeviceN above 16 components emits `color.devicen.component_cap`;
- missing tint transforms emit structured diagnostics.

Screen preview uses the PDF tint transform into the alternate color space. This
is acceptable for preview but is not a claim of true spot-plate output. Prompt
05B makes the preview posture explicit in report fields: Separation/DeviceN
space counts, tint-transform counts, missing-transform counts, and the preview
model string.

Prompt 12 adds a sparse separation framebuffer side-channel. Separation and
DeviceN colorant names, tint values, alternate preview RGB, alpha, and
provenance are preserved in
`prompt12_prepress_cmm_device_link_separation_plates`. RGB page output remains a
visual preview. The plate report is the source of truth for Prompt 12
separation evidence.

## Overprint

Prompt 05 parses and preserves:

- `/OP` stroke overprint;
- `/op` fill overprint;
- `/OPM` overprint mode.

The graphics-state stack restores these values across `q`/`Q`, and display-list
`DrawState` carries them for future device backends. Prompt 05B adds a bounded
CPU preview for DeviceCMYK fills: when fill overprint is enabled, zero source
ink channels preserve the approximate destination CMYK channel before returning
to the RGB framebuffer. The report still emits
`color.overprint.preview_approximation` because this is not true prepress plate
compositing, and stroke/spot/DeviceN overprint remain diagnostic or
alternate-space preview only.

## ICC Backend and BPC

Prompt 05B keeps the default backend as safe Rust plus qcms. ICC transforms are
cached under a bounded entry count and the report exposes transform cache
metrics and sRGB identity fidelity probes. Rendering intent values are parsed
and reported; qcms-supported intent names are exposed. Black-point compensation
is still reported as unavailable in the default backend rather than silently
pretended.

Prompt 12 extends ICC inventory with profile class detection for input,
display, output, device-link, color-space conversion, abstract, named-color,
malformed, and unsupported profiles. Device-link and multicolor transforms that
cannot be executed safely are reported as unsupported rather than flattened into
RGB proofing.

## Standards Boundary

Color validation now catches missing/malformed output intents, ICC profile
decode/basic-parse failures, overlarge ICC profiles, spot/DeviceN metadata, and
overprint usage, plus the Prompt 05B color-specific PDF/A and PDF/X
output-intent checks above. It does not yet validate every PDF/A or PDF/X
requirement: metadata schemas, annotation constraints, font embedding,
transparency rules, image compression requirements, and full prepress
separations remain in their own standards phases.

Prompt 12 does not change that standards boundary. It improves prepress
structure preservation and reporting, but it is not certification-grade PDF/X
validation. Prompt 13 reports bounded overprint simulation, but it still does
not claim certification-grade PDF/X validation.
