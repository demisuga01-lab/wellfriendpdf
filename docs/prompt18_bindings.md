# Prompt 18 bindings

Rust owns all models and mutation code. The CLI calls `oxide_engine::sdk`; it has `redact-image-mask`, `redact-inline-image`, `associated-files-report`, `associated-files-extract`, `associated-files-add`, `associated-files-remove`, `associated-files-sanitize`, `edit-signature-impact`, `edit-policy-report`, and `prompt18-report`.

Python returns owned dictionaries/bytes and raises Oxide exceptions. The C ABI returns versioned JSON and owned `OxideBuffer` values with existing explicit free functions. WASM accepts and returns memory bytes only and performs no host path or external access. .NET `IDisposable` and Java `AutoCloseable` documents expose Prompt 18 reports plus mask/inline redaction and associated-file add/sanitize byte operations through the same C ABI.
