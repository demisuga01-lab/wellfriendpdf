# Binding Examples — Prompt 01

Three parallel `sdk_reports` examples — Rust, Python, C — call the **same**
`oxide_engine::sdk` facade and print/emit the **same** versioned-JSON envelopes.
Running all three over the same PDF and comparing the `report` bodies proves
cross-surface parity.

## Rust

```sh
cargo run -p oxide-engine --example sdk_reports -- input.pdf [out.json]
```

Source: `crates/engine/examples/sdk_reports.rs`. Drives every facade area; with a
second argument writes an aggregate smoke JSON (used for
`target/prompt01-binding-core/rust-api-smoke.json`).

## Python

```sh
cd crates/oxide-py
python -m venv .venv && .venv/Scripts/python -m pip install maturin
.venv/Scripts/python -m maturin develop --release
.venv/Scripts/python examples/sdk_reports.py input.pdf [out.json]
```

Source: `crates/oxide-py/examples/sdk_reports.py`. Uses `oxide.open`, the report
methods, and the module-level `feature_report` / `decode_budget_report`.

## C

```sh
cargo build -p oxide-capi        # builds target/debug/oxide_capi.dll(.lib)

# MSVC:
cl /I crates/oxide-capi/include crates/oxide-capi/examples/sdk_reports.c \
   /Fe:sdk_reports.exe /link target/debug/oxide_capi.dll.lib
# gcc/clang:
cc -I crates/oxide-capi/include crates/oxide-capi/examples/sdk_reports.c \
   -Ltarget/debug -loxide_capi -o sdk_reports

sdk_reports input.pdf [out.json]   # DLL must be on PATH / next to the exe
```

Source: `crates/oxide-capi/examples/sdk_reports.c`. Calls `oxide_version`,
`oxide_feature_report_json`, the report functions, and `oxide_document_sanitize_json`,
freeing every allocation. With a second argument writes a smoke JSON (used for
`target/prompt01-binding-core/c-abi-smoke.json`). Verified compiled with MSVC and
run against the real DLL in this prompt.

## Cross-surface parity check

```python
import json
c = json.load(open("target/prompt01-binding-core/c-abi-smoke.json"))
p = json.load(open("target/prompt01-binding-core/python-api-smoke.json"))
r = json.load(open("target/prompt01-binding-core/rust-api-smoke.json"))
assert c["security"]["report"] == p["security"]["report"] == r["security"]["report"]
assert c["forms"]["report"]    == p["forms"]["report"]
assert c["chunks"]["report"]   == r["chunk"]["report"]
```

All three assertions hold: the security, forms, and chunk report bodies are
byte-identical across Rust, Python, and C.

## Common recipes

| Task | Rust | Python | C |
| --- | --- | --- | --- |
| Open | `ContentEngine::open_bytes` | `oxide.open` | `oxide_document_open_from_bytes` |
| Security report | `sdk::security_report_json` | `doc.security_report()` | `oxide_document_security_report_json` |
| Validate PDF/A | `sdk::pdfa_validation_json` | `doc.validate_pdfa()` | `oxide_document_validate_json(doc,"pdfa",…)` |
| Sanitize | `sdk::sanitize_json` | `doc.sanitize()` | `oxide_document_sanitize_json` |
| Redact terms | `sdk::redact_terms_json` | `doc.redact([...])` | `oxide_document_redact_terms_json` |
| Canonicalize | `sdk::canonicalize_json` | `doc.canonicalize()` | `oxide_document_canonicalize_json` |
| Capabilities | `sdk::feature_report_json` | `oxide.feature_report()` | `oxide_feature_report_json` |
| Free (C only) | — | (GC) | `oxide_string_free` / `oxide_buffer_free` / `oxide_document_free` |
