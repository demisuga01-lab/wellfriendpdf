# Prompt 11B Native CMM Safety Audit

Backend decision: implement LittleCMS/lcms2 through the Rust `lcms2` crate,
behind the explicit `native-cmm-lcms2` feature.

Boundary:

- `wellfriendpdf-engine` keeps `#![forbid(unsafe_code)]`.
- Unsafe/native FFI is isolated in the external `lcms2` and `lcms2-sys`
  dependencies.
- The default engine build has no LittleCMS dependency.
- WASM builds do not enable native CMM.

Dependency and linking:

- Rust crates: `lcms2 = 6.1.1`, `lcms2-sys = 4.0.7`.
- Crate license: MIT for the Rust wrapper/sys crates.
- Native library: LittleCMS/lcms2.
- Discovery: `LCMS2_LIB_DIR`, pkg-config, or `lcms2-sys` static fallback.
- Packaging: language packages must not claim or bundle native CMM unless their
  native library was explicitly built with `native-cmm-lcms2`.

Security controls:

- ICC profiles are attacker-controlled input and are size-capped to 16 MiB.
- Malformed profiles return no transform and increment diagnostics/metrics.
- Profile channel counts are validated against PDF `/N`.
- Transform cache is bounded to 16 entries by default.
- Native backend unavailability is reported, not panicked.

Prompt 12 adds device-link and multicolor inventory, sparse separation
framebuffer reporting, and spot/DeviceN plate preservation on top of this
boundary. It keeps the same native dependency rule: default and WASM builds do
not link LittleCMS, and native behavior remains behind `native-cmm-lcms2`.
Prompt 13 closes bounded overprint/prepress simulation. Certification-grade
PDF/X validation remains later standards work.
