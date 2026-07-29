# Native CMM Backend CMM Backend Selection

Backend selection is compile-time explicit:

- Default build: qcms/default fallback.
- `native-cmm-lcms2`: LittleCMS/lcms2 for ICCBased profile transforms on native
  targets.
- WASM: native CMM unavailable; fallback remains active.

The selected backend appears in `feature_report_json()` under
`native_cmm_backend_native_littlecms_cmm_backend_closure.backend_selected`.

No report surface may claim `lcms2` unless the current build was compiled with
`native-cmm-lcms2` and is running on a native target. Default Python, .NET, Java,
WASM, and CLI packages therefore report fallback unless they are rebuilt with
the native feature.
