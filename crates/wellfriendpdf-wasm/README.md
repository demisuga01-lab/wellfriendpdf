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
  `powershell -ExecutionPolicy Bypass -File scripts\wasm_packaging_wasm_pack_gate.ps1`
  from the repository root. The gate bootstraps target-local `wasm-pack 0.13.1`,
  builds web and Node package directories, inspects contents, and runs a
  packaged Node smoke.
- Direct package commands remain:
  `wasm-pack build crates/wellfriendpdf-wasm --target web --out-dir pkg` for
  browser/WebWorker use and
  `wasm-pack build crates/wellfriendpdf-wasm --target nodejs --out-dir pkg-node` for
  Node use.
- `wellfriendpdf.d.ts` documents the Binding Parity/03 public TypeScript surface.

The checked-in browser example under `examples/browser` must be regenerated
after source changes; its prebuilt `pkg/` directory is an example artifact, not
the source of truth.

## Supported Operations

Public report methods include document info, security, risky-content, parser,
color, validation, forms, annotations, page operations, interactive content,
signature, font, semantic text, semantic document, chunks, image decode
capability/lifecycle, and decode-budget reports, plus
`WellfriendPdf.codecIsolationReportJson(filter, bytes, policy)` for
Release Packaging codec policy diagnostics. Output-producing methods include
`sanitize`, `canonicalize`, and `redactTermsJson`.

Semantic Closeout adds `semanticBundleJson`, `advancedChunksJson`,
`semanticSearchJson`, and static `tableProposalStatusJson`. These are local,
byte-only browser surfaces. They do not assume a filesystem or native ML
runtime and do not upload input.

Legacy parser and extraction methods remain available: `parseJson`,
`parseMarkdown`, `chunk`, `extractText`, `extractStructuredText`,
`extractFieldsJson`, and `renderPagePng`.

Editing transaction apply can return the shared render-invalidation plan through
`editing_transactionsTransactionApplyWithRenderInvalidation(requestJson,
renderInvalidationOptionsJson)`, including mapped source IDs and optional dirty
render tiles for caller-side cache invalidation.

`RenderCache` exposes caller-owned render-cache handles in JavaScript. Contract
PNG renders can use `renderContractPngWithRenderCache` or
`renderContractPngWithRenderCacheReport`, and cache owners can apply the
SDK/server `report.render_invalidation` JSON with
`RenderCache.applyRenderInvalidationPlanJson`.

Image decode lifecycle reporting is available through
`progressiveImageDecodeLifecycleReportJson(requestJson)`, which returns bounded
start/continue/pause/resume/cancel/fail/close/document_close reports for a
discovered image without decoding pixels when current codec adapters report
full-decode-only.

Contract rendering exposes cooperative cancellation through `RenderCancellation`
for JSON and typed-object PNG renders, report-returning renders, caller-owned
buffer renders, and caller-owned buffer report renders. Existing compatibility
methods remain non-cancellable.

## Ownership and Limits

Input bytes remain owned by the JavaScript caller; the WASM object keeps its own
copy so report methods can reopen through the shared facade. Output bytes are
returned as fresh `Uint8Array` values owned by JavaScript. `close()` marks the
document closed; further calls return an exception instead of silently reusing a
dead handle.

The WASM surface does not read host file paths, fetch URLs, spawn OCR processes,
write output files, or expose native library loading. Progressive jobs expose
`reviseRenderContractJson(contractJson)` for full live schema-v1 contract
revision, plus `requestCancel`,
`stepWithCancellation(maxTiles, booleanOrAbortSignal)`, and
`finishPngWithCancellation(booleanOrAbortSignal)` for synchronous pre/post
cancellation checks. `finishPng()` and the cancellation variant surface checked
tile-assembly diagnostics if called before all tiles are complete, plus
`viewerQueueJson()` and
`executeViewerQueueJson(maxItems)` for source-visible viewer scheduling and
owned current-page queue execution, `executeAdjacentPagePrefetch(prefetchIdentity,
maxTiles)` for bounded adjacent-page child-job prefetch execution,
`executeViewerQueueJsonWithCancellation(maxItems, cancellation)` and
`executeAdjacentPagePrefetchWithCancellation(prefetchIdentity, maxTiles,
cancellation)` for cancellable queue/prefetch execution, plus
`viewerCallbackDispatchJson()` and `dispatchViewerCallbacks(callback)` for
synchronous host callback execution. External viewer runtime matrices and
broader cross-language queue policy validation are deferred verification work.

Subprocess codec isolation is not available in WASM because the target cannot
spawn the OS codec worker. Use `policy = "in_process"` for browser/Node local
decode reports; fail-closed subprocess policies return structured reports.
