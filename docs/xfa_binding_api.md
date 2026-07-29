# XFA binding API

The Rust SDK and shared JSON facade expose inventory, extraction, script/security/runtime reports, render preview, flatten, and sanitize. CLI commands are `xfa-report`, `xfa-extract`, `xfa-script-report`, `xfa-security-report`, `xfa-runtime-report`, `xfa-render`, `xfa-flatten`, and `xfa-sanitize`.

Python, C ABI, WASM, .NET, Java Maven, and Java Gradle wrap the same facade. `xfa-security-report` is an additive CLI view of the same security report. Reports use envelope version 1 and inner schema `xfa_runtime.xfa.v1`. Output operations return owned PDF bytes plus an owned report and optionally write an explicit file. C callers free strings/buffers with the documented Wellfriend free functions; managed bindings copy into owned values.

No binding exposes an XML DOM pointer, script handle, interpreter global, or external-resource callback. Unsupported and security-policy statuses are serialized verbatim. Default limits are present in reports. The feature report adds `xfa_runtime_xfa_runtime_sandbox_closure` without changing prior sections.
