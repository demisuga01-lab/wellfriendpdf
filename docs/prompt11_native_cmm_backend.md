# Prompt 11 Native CMM Backend

Prompt 11 does not implement a LittleCMS native backend. It records a precise
hard block and keeps the existing qcms/default color path active and reported.

## Implemented in the Current Build

- ICC profile load limits: 16 MiB profile cap.
- Invalid ICC behavior: fail-closed diagnostics/reporting.
- ICCBased image/profile preview: qcms profile-to-sRGB where accepted.
- DeviceRGB behavior: direct sRGB preview path.
- DeviceCMYK behavior: deterministic process-ink preview.
- CalRGB/CalGray/Lab behavior: existing fallback conversions.
- Rendering intent: parsed/reported and carried where qcms supports it.
- Caches: transform/profile keys include profile data, intent posture, and
  color-space role in the current bounded cache.

## Not Claimed

- LittleCMS native transforms
- device-link ICC
- multicolor ICC
- true black-point compensation
- output-intent destination proofing
- separation or DeviceN plate framebuffers
- overprint proofing
- full prepress parity

## Report Surface

The shared feature report section is:

```text
prompt11_renderer_fuzz_cmm_closeout
```

It exposes the current backend as `safe-rust-plus-qcms`, records the native CMM
status as hard-blocked, and keeps default/WASM native dependency fields false.
