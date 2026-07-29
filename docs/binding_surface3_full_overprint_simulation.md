# Prepress Proofing Full Overprint Simulation

Prepress Proofing adds a bounded overprint state model to the existing prepress plate
pipeline. The model carries fill overprint `op`, stroke overprint `OP`, OPM,
paint role, color space, component values, alpha, object provenance, and native
CMM/output-intent context into plate contributions and cache fingerprints.

Implemented:

- `OP` and `op` are distinct graphics-state values.
- `OPM 0` and `OPM 1` are normalized into process replacement or zero-tint
  preservation behavior for supported paths.
- DeviceCMYK fill, stroke, and fill+stroke overprint preview uses the process
  overprint compositor.
- Separation and DeviceN plate contributions preserve spot/process distinction,
  tint, alpha, operation, page, object provenance, and overprint posture.
- Text, vector, image, shading, and tiling-pattern plate paths write Prepress Proofing
  posture rows where their Nchannel Plate Prepress plate path is representable.
- Knockout/replacement rows are explicit when overprint is disabled.

Exact limits:

- Vendor-specific RIP behavior is not modeled without reference evidence.
- Resource-heavy Type3 charprocs that invoke nested XObjects, images, shadings,
  or patterns remain fail-closed until recursive Type3 resource execution owns
  those resources.
- Unsafe high-channel image or ICC pixel formats that the safe native wrapper
  cannot expose are `unsupported_reported_exact`.
- Certification-grade PDF/X validation is not part of Prepress Proofing.
