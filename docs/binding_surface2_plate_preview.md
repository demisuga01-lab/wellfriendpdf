# Prepress CMM Plate Preview

Prepress CMM exposes plate preview evidence through report hashes and artifacts.
The primary artifact is:

```text
target/prepress_cmm-prepress-cmm/plate-preview-results-prepress_cmm.json
```

The preview hashes prove deterministic per-plate data exists without claiming a
press-calibrated raster plate export. RGB page output remains a visual preview.

The plate preview report includes:

- output mode
- preview hash count
- per-plane preview hashes
- provenance fields
- Prepress Proofing overprint posture

Full per-plate image export can build on this report model later. Prepress CMM's
acceptance target is preservation and visibility of plate state, not a
certified separations export workflow.
