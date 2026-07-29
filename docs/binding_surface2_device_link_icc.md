# Prepress CMM Device-Link ICC

Device-link ICC profiles are detected from the ICC profile class signature
`link`. The inventory keeps the profile hash, object reference when available,
profile byte size, source color-space signature, PCS/output signature, input and
output channel counts, rendering intent hint, and native/fallback status.

Native behavior:

- `native-cmm-lcms2` is required for device-link execution.
- Device-link profiles are treated as fixed transforms, not ordinary source
  profiles to combine blindly with another destination profile.
- Legal source/output channel shapes may use the LittleCMS path.
- Nchannel Plate Prepress adds the n-channel intermediate representation needed to carry
  device-link output channels into plate/prepress reporting when the safe native
  wrapper exposes the needed shape.
- Channel mismatch, unsupported channel shapes, malformed profiles, oversized
  profiles, and ambiguous output-intent relationships fail closed with
  diagnostics.

Fallback behavior:

- default and WASM builds do not claim device-link proofing.
- fallback reports device-link inventory plus unsupported transform status.
- alternate color-space output may be preview-only when the PDF supplies a safe
  alternate; it is not reported as device-link proof.

Output intents:

- a device-link profile is already a source-to-destination transform, so Roadmap task
  12 reports `do_not_double_proof` style posture for ambiguous output-intent
  combinations.
- ordinary source profiles may still use the Native CMM Backend output-intent proofing
  path when the relationship is clear.

Nchannel Plate Prepress limit:

- if the native LittleCMS wrapper cannot expose the profile's exact input/output
  pixel format safely, the profile is inventoried and reported as unsupported
  for transform execution instead of being flattened to RGB.
