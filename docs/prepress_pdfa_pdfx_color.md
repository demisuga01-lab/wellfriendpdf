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
  `DestOutputProfile`.
- `PdfX`: same color-output-intent checks as `PdfA`; broader PDF/X rules remain
  outside this Prompt 05 subset.

## Spot Colors and DeviceN

Separation and DeviceN are preserved in report metadata:

- spot colorant names are listed under `spot_colorants`;
- DeviceN component arrays are listed under `devicen_components`;
- DeviceN above 16 components emits `color.devicen.component_cap`;
- missing tint transforms emit structured diagnostics.

Screen preview uses the PDF tint transform into the alternate color space. This
is acceptable for preview but is not a claim of true spot-plate output.

## Overprint

Prompt 05 parses and preserves:

- `/OP` stroke overprint;
- `/op` fill overprint;
- `/OPM` overprint mode.

The graphics-state stack restores these values across `q`/`Q`, and display-list
`DrawState` carries them for future device backends. Current CPU screen preview
emits `color.overprint.preview_approximation` because it does not simulate true
prepress plate compositing.

## Standards Boundary

Color validation now catches missing/malformed output intents, ICC profile
decode/basic-parse failures, overlarge ICC profiles, spot/DeviceN metadata, and
overprint usage. It does not yet validate every PDF/A or PDF/X requirement:
metadata schemas, annotation constraints, font embedding, transparency rules,
image compression requirements, and full prepress separations remain in their
own standards phases.
