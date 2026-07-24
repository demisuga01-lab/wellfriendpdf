# Prompt 11B Native LittleCMS CMM Backend Closure

Prompt 11B closes the native CMM gap left by Prompt 11. Wellfriend now has an
explicit `native-cmm-lcms2` feature that compiles the safe Rust `lcms2` wrapper
and links LittleCMS/lcms2 through `lcms2-sys`.

Default builds do not enable or link this backend. WASM builds report native CMM
unavailable and continue to use the portable qcms/default color path.

Implemented behavior:

- ICCBased Gray, RGB, and CMYK profile-to-sRGB preview transforms through
  LittleCMS when `native-cmm-lcms2` is enabled.
- Real CMYK transform coverage using the ICC PRMG CMYK fixture
  `tests/fixtures/icc/PRMG_v2.0.1_MR.icc`.
- Malformed ICC and channel-count mismatch fail-closed behavior.
- A 16 MiB ICC profile size cap before profile parsing.
- Transform cache keys that include backend, profile hash, profile length,
  source channel count, pixel formats, rendering intent, and BPC posture.
- Basic output-intent soft-proofing helper through LittleCMS when the native
  backend is active.
- Additive feature-report section
  `prompt11b_native_littlecms_cmm_backend_closure`.

Not claimed:

- Certification-grade PDF/X proofing.
- Device-link ICC execution as a general render feature.
- Multicolor ICC and n-color output.
- True separation framebuffers and spot/DeviceN plate preview are Prompt 12/12B
  owners. Bounded overprint/prepress simulation is closed by Prompt 13.

Those are Prompt 12/13 CMM and prepress owners.
