# Binding Examples — Prompt 01

Three parallel `sdk_reports` examples — Rust, Python, C — call the **same**
`wellfriendpdf_engine::sdk` facade and print/emit the **same** versioned-JSON envelopes.
Running all three over the same PDF and comparing the `report` bodies proves
cross-surface parity.

## Rust

```sh
cargo run -p wellfriendpdf-engine --example sdk_reports -- input.pdf [out.json]
```

Source: `crates/engine/examples/sdk_reports.rs`. Drives every facade area; with a
second argument writes an aggregate smoke JSON (used for
`target/prompt01-binding-core/rust-api-smoke.json`).

## Python

```sh
cd crates/wellfriendpdf-py
python -m venv .venv && .venv/Scripts/python -m pip install maturin
.venv/Scripts/python -m maturin develop --release
.venv/Scripts/python examples/sdk_reports.py input.pdf [out.json]
```

Source: `crates/wellfriendpdf-py/examples/sdk_reports.py`. Uses `wellfriendpdf.open`, the report
methods, and the module-level `feature_report` / `decode_budget_report`.

## C

```sh
cargo build -p wellfriendpdf-capi        # builds target/debug/wellfriendpdf_capi.dll(.lib)

# MSVC:
cl /I crates/wellfriendpdf-capi/include crates/wellfriendpdf-capi/examples/sdk_reports.c \
   /Fe:sdk_reports.exe /link target/debug/wellfriendpdf_capi.dll.lib
# gcc/clang:
cc -I crates/wellfriendpdf-capi/include crates/wellfriendpdf-capi/examples/sdk_reports.c \
   -Ltarget/debug -lwellfriendpdf_capi -o sdk_reports

sdk_reports input.pdf [out.json]   # DLL must be on PATH / next to the exe
```

Source: `crates/wellfriendpdf-capi/examples/sdk_reports.c`. Calls `wellfriendpdf_version`,
`wellfriendpdf_feature_report_json`, the report functions, and `wellfriendpdf_document_sanitize_json`,
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
| Open | `ContentEngine::open_bytes` | `wellfriendpdf.open` | `wellfriendpdf_document_open_from_bytes` |
| Security report | `sdk::security_report_json` | `doc.security_report()` | `wellfriendpdf_document_security_report_json` |
| Validate PDF/A | `sdk::pdfa_validation_json` | `doc.validate_pdfa()` | `wellfriendpdf_document_validate_json(doc,"pdfa",…)` |
| Sanitize | `sdk::sanitize_json` | `doc.sanitize()` | `wellfriendpdf_document_sanitize_json` |
| Redact terms | `sdk::redact_terms_json` | `doc.redact([...])` | `wellfriendpdf_document_redact_terms_json` |
| Canonicalize | `sdk::canonicalize_json` | `doc.canonicalize()` | `wellfriendpdf_document_canonicalize_json` |
| Capabilities | `sdk::feature_report_json` | `wellfriendpdf.feature_report()` | `wellfriendpdf_feature_report_json` |
| Free (C only) | — | (GC) | `wellfriendpdf_string_free` / `wellfriendpdf_buffer_free` / `wellfriendpdf_document_free` |
