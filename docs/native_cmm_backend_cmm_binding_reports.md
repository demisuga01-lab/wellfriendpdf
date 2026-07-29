# Native CMM Backend CMM Binding Reports

All binding surfaces consume the shared SDK report section:

`native_cmm_backend_native_littlecms_cmm_backend_closure`

The section includes:

- native CMM compiled
- native CMM runtime availability
- selected backend
- native backend version/crate versions
- feature flag status
- profile size and transform cache caps
- output-intent proofing status
- rendering-intent and BPC status
- WASM and package posture
- exact remaining limits

Rust, CLI, Python, C ABI, WASM, .NET, Java Maven, and Java Gradle must expose
the same backend truth. Fallback builds must not claim `lcms2`.
