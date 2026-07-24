# Prompt 12 Backend Packaging Policy

Default portability remains the packaging rule:

- default builds use the portable fallback CMM posture
- WASM must not pull native LittleCMS
- native LittleCMS code remains behind `native-cmm-lcms2`
- `wellfriendpdf-engine` stays free of unsafe code
- unsafe/native integration remains isolated in dependency crates

Prompt 12B reports native and fallback behavior through the same additive public
schema. Native builds may report LittleCMS-backed device-link and n-channel
posture where the profile context and safe pixel format are available. Fallback
and WASM builds report no-native-backend for device-link and multicolor ICC
proofing and may expose only clearly labeled preview/inventory behavior.

Packaging and binding smokes must assert the Prompt 12B report section across:

- Rust SDK
- CLI
- Python
- C ABI
- WASM
- .NET
- Java Maven
- Java Gradle

No package or binding may claim native n-channel transform support unless the
binary is built with the native feature and the runtime report says the native
backend is active.
