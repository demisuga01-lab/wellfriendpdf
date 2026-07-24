# Wellfriend WASM SDK

`wellfriendpdf-wasm` is the browser, Node, and WebWorker surface for the shared
`wellfriendpdf_engine::sdk` facade. It accepts caller-owned PDF bytes and returns the
same versioned JSON report envelopes as Rust, Python, and the C ABI.

```ts
import init, { WellfriendPdf } from "@wellfriendpdf/wellfriendpdf-wasm";

await init();
const pdf = new WellfriendPdf(await file.arrayBuffer());
const security = JSON.parse(pdf.securityReportJson());
const semantic = JSON.parse(pdf.semanticBundleJson());
const chunks = JSON.parse(pdf.advancedChunksJson());
const search = JSON.parse(pdf.semanticSearchJson("invoice"));
const sanitized = pdf.sanitize("balanced");

console.log(pdf.pageCount(), security.status);
console.log(sanitized.byteLength(), sanitized.reportJson());
pdf.close();
```

## Package Shape

- For release evidence, run
  `powershell -ExecutionPolicy Bypass -File scripts\prompt03b_wasm_pack_gate.ps1`
  from the repository root. The gate bootstraps target-local `wasm-pack 0.13.1`,
  builds web and Node package directories, inspects contents, and runs a
  packaged Node smoke.
- Direct package commands remain:
  `wasm-pack build crates/wellfriendpdf-wasm --target web --out-dir pkg` for
  browser/WebWorker use and
  `wasm-pack build crates/wellfriendpdf-wasm --target nodejs --out-dir pkg-node` for
  Node use.
- `wellfriendpdf.d.ts` documents the Prompt 02/03 public TypeScript surface.

The checked-in browser example under `examples/browser` must be regenerated
after source changes; its prebuilt `pkg/` directory is an example artifact, not
the source of truth.

## Supported Operations

Public report methods include document info, security, risky-content, parser,
color, validation, forms, annotations, page operations, interactive content,
signature, font, semantic text, semantic document, chunks, and decode-budget
reports, plus `WellfriendPdf.codecIsolationReportJson(filter, bytes, policy)` for
Prompt 03 codec policy diagnostics. Output-producing methods include
`sanitize`, `canonicalize`, and `redactTermsJson`.

Prompt 15 adds `semanticBundleJson`, `advancedChunksJson`,
`semanticSearchJson`, and static `tableProposalStatusJson`. These are local,
byte-only browser surfaces. They do not assume a filesystem or native ML
runtime and do not upload input.

Legacy parser and extraction methods remain available: `parseJson`,
`parseMarkdown`, `chunk`, `extractText`, `extractStructuredText`,
`extractFieldsJson`, and `renderPagePng`.

## Ownership and Limits

Input bytes remain owned by the JavaScript caller; the WASM object keeps its own
copy so report methods can reopen through the shared facade. Output bytes are
returned as fresh `Uint8Array` values owned by JavaScript. `close()` marks the
document closed; further calls return an exception instead of silently reusing a
dead handle.

The WASM surface does not read host file paths, fetch URLs, spawn OCR processes,
write output files, or expose native library loading. Progress callbacks and
cancellation tokens are not advertised because the current facade calls are
synchronous and do not observe binding-level cancellation.

Subprocess codec isolation is not available in WASM because the target cannot
spawn the OS codec worker. Use `policy = "in_process"` for browser/Node local
decode reports; fail-closed subprocess policies return structured reports.
