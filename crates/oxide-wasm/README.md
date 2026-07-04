# Oxide WASM SDK

`oxide-wasm` is the browser, Node, and WebWorker surface for the shared
`oxide_engine::sdk` facade. It accepts caller-owned PDF bytes and returns the
same versioned JSON report envelopes as Rust, Python, and the C ABI.

```ts
import init, { OxidePdf } from "@oxidepdf/oxide-wasm";

await init();
const pdf = new OxidePdf(await file.arrayBuffer());
const security = JSON.parse(pdf.securityReportJson());
const sanitized = pdf.sanitize("balanced");

console.log(pdf.pageCount(), security.status);
console.log(sanitized.byteLength(), sanitized.reportJson());
pdf.close();
```

## Package Shape

- Build with `wasm-pack build crates/oxide-wasm --target web --out-dir pkg` for
  browser/WebWorker use.
- Build with `wasm-pack build crates/oxide-wasm --target nodejs --out-dir pkg-node`
  for Node use.
- `oxide.d.ts` documents the Prompt 02 public TypeScript surface.

The checked-in browser example under `examples/browser` must be regenerated
after source changes; its prebuilt `pkg/` directory is an example artifact, not
the source of truth.

## Supported Operations

Public report methods include document info, security, risky-content, parser,
color, validation, forms, annotations, page operations, interactive content,
signature, font, semantic text, semantic document, chunks, and decode-budget
reports. Output-producing methods include `sanitize`, `canonicalize`, and
`redactTermsJson`.

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
