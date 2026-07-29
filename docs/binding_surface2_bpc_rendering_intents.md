# Prepress CMM BPC And Rendering Intents

Wellfriend reports and cache-keys all four ICC rendering intents:

- perceptual
- relative colorimetric
- saturation
- absolute colorimetric

Invalid intent values are reported and resolved through the documented default
policy rather than silently disappearing.

Native BPC:

- `native-cmm-lcms2` may pass the LittleCMS black-point compensation flag when
  the transform requests BPC.
- BPC state is part of the transform cache key.
- unsupported transform combinations fail closed.

Fallback BPC:

- default and WASM builds report `bpc_unsupported_in_fallback`.
- fallback output remains preview, not proof.

Cache posture:

- transform cache keys include backend, profile hash, channel count, rendering
  intent, and BPC state.
- render cache keys now include a prepress fingerprint derived from Separation
  and DeviceN color-space state.
