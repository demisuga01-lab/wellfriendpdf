# WASM SDK Prompt 02

The WASM SDK is built from `crates/oxide-wasm` and targets browser, Node, and
WebWorker environments. It accepts caller-provided bytes and routes report and
output operations through `oxide_engine::sdk`.

## Package Shape

- Rust crate: `crates/oxide-wasm`
- JS package metadata: `crates/oxide-wasm/package.json`
- TypeScript declarations: `crates/oxide-wasm/oxide.d.ts`
- Browser example: `crates/oxide-wasm/examples/browser`

Build:

```sh
cargo build -p oxide-wasm --target wasm32-unknown-unknown
wasm-pack build crates/oxide-wasm --target web --out-dir pkg
```

`cargo build` verifies the Rust/WASM code. `wasm-pack` or `wasm-bindgen` must
regenerate JS glue before publishing or using newly added methods.

## Public API

Lifecycle and capability queries:

- `new OxidePdf(bytes)`
- `OxidePdf.openWithPassword(bytes, password)`
- `close()`, `isClosed()`
- `OxidePdf.sdkVersion()`, `OxidePdf.abiVersion()`
- `OxidePdf.featureReportJson()`
- `OxidePdf.decodeBudgetReportJson(filter, width, height, components)`

Reports:

- `documentInfoJson`
- `securityReportJson`
- `riskyContentReportJson`
- `parserReportJson`
- `colorReportJson`
- `validateJson`, `validatePdfaJson`, `validatePdfuaJson`
- `formsReportJson`, `annotationsReportJson`, `pagesReportJson`
- `interactiveReportJson`
- `signatureReportJson`, `fontReportJson`
- `textSemanticJson`, `semanticDocumentReportJson`, `chunksJson`

Outputs:

- `sanitize(policy?)`
- `canonicalize(dateEpoch?)`
- `redactTermsJson(termsJson, strict)`

Legacy extraction/render methods remain public: `parseJson`, `parseMarkdown`,
`chunk`, `extractText`, `extractStructuredText`, `extractSemanticJson`,
`extractFieldsJson`, `infoJson`, and `renderPagePng`.

## Ownership and Limits

Input bytes are copied into the WASM object so facade calls can reopen through
the same byte source. Output bytes are returned as fresh JS-owned `Uint8Array`
values. A closed document rejects future calls.

WASM does not fetch URLs, read host file paths, write host files, spawn OCR, or
load native binaries. Prompt 02B closes progress/cancellation posture through
the shared feature report: `progress_not_supported` and
`cancellation_not_supported_for_prompt02_bindings` are reported, and the WASM
SDK does not expose fake callbacks or ignored `AbortSignal` options. Those
entries are matrixed as unsupported or partial instead of hidden behind no-op
wrappers.
