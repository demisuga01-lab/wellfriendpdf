# Prompt 11B CMM Packaging Policy

Native CMM is never silently enabled.

Rust and CLI:

- Source builds may enable `native-cmm-lcms2`.
- Default builds do not link LittleCMS.

Python:

- The default wheel reports fallback/qcms.
- A native-CMM wheel must be intentionally built with `native-cmm-lcms2` and
  must document whether LittleCMS is bundled or dynamically resolved.

C ABI, .NET, Java Maven, Java Gradle:

- Native libraries must report whether they were compiled with LittleCMS.
- Package smokes must not claim `lcms2` when fallback is active.

WASM:

- Native CMM is unavailable.
- The fallback report remains visible.
