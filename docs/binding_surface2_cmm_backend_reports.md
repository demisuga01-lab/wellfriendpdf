# Prepress CMM CMM Backend Reports

Prepress CMM adds the feature-report section:

```text
prepress_cmm_prepress_cmm_device_link_separation_plates
```

The section is additive and is exposed through Rust SDK, CLI, Python, C ABI,
WASM, .NET, and Java smoke surfaces.

Required backend posture:

- default builds report fallback/qcms preview and no native LittleCMS.
- WASM reports native CMM unavailable.
- native builds report `native-cmm-lcms2` and selected LittleCMS backend when
  compiled and available.
- bindings expose the same JSON envelope and do not claim native CMM unless the
  underlying native library was built with the feature.

Color/prepress reports also expose ICC profile class, profile hash, channel
counts, output-intent class/channel details, BPC/rendering-intent posture, and
separation plate summaries.
