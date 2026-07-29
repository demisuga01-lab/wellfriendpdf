# Prepress CMM Known Limits

Prepress CMM and Nchannel Plate Prepress intentionally did not claim full overprint
compositing. roadmap closure 13 now owns the bounded overprint/prepress
close-out. Prepress CMM remains the device-link, multicolor ICC, BPC/intent, and
sampled separation framebuffer baseline.

Prepress CMM and Nchannel Plate Prepress still do not claim:

- certification-grade PDF/X validation; a later standards phase owns that.
- press-calibrated per-plate raster export for every production workflow.

Exact remaining limits after Nchannel Plate Prepress:

- resource-heavy Type3 charprocs that invoke nested XObjects, shadings, or
  images are report-visible until recursive Type3 resource execution owns those
  resources.
- high-channel ICC profiles whose exact n-channel pixel format is not exposed by
  the safe LittleCMS wrapper are inventoried and reported as
  `unsupported_reported_unsafe_profile` rather than transformed.
- fallback/default/WASM builds do not perform device-link or multicolor
  n-channel proofing; they report inventory/preview-only behavior.
- unsafe packed image layouts with excessive channel counts fail closed or
  degrade to report-only plate diagnostics.
- vendor-specific RIP overprint behavior not evidenced by Prepress Proofing references
  is not claimed.

Unsupported paths must not be silent. Unsupported profile classes, malformed
profiles, channel mismatches, excessive colorants, missing alternates, recursive
patterns, and unsafe tint transforms must produce report rows or diagnostics.
