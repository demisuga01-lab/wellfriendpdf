# Wellfriend WASM SDK

`wellfriendpdf-wasm` is the browser, Node, and WebWorker surface for the shared
`wellfriendpdf_engine::sdk` facade. It accepts caller-owned PDF bytes and returns the
same versioned JSON report envelopes as Rust, Python, and the C ABI.

For the new retained native-editing session and embeddable worker-backed linked
story editor, see [browser editing](browser/README.md). The Rust crate compiles
and the checked-in TypeScript declaration surface type-checks. It has not been
rendered or browser-tested and is not part of a generated WASM package until the
artifact is rebuilt.

`StoryEditSession.openWithPassword(bytes, passwordBytes)` unlocks a
Standard-handler encrypted input and retains only the explicitly reported
unencrypted working revision. Exact password bytes are borrowed for synchronous
open and are not retained. The credential must authenticate as the
permissions/owner password; user/open passwords are rejected. Callers that need encrypted output must request it
explicitly through the universal publication API.

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

### ECBES universal transaction

`pdf.ecbesUniversalEdit(requestJson)` materializes bounded universal-edit
candidates privately and returns only the evidence-qualified `WellfriendOutput`.
`ecbesUniversalEditWithOutputCredentials` supports Standard-security candidates
with apply-only `Uint8Array` credentials. If no candidate qualifies, the result
contains the exact original transport bytes. See the
[ECBES specification](../../docs/research/evidence_constrained_bidirectional_edit_synthesis.md).

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

### Native image movement (source-only, unqualified)

`previewImageFragmentMoveJson(bytes, requestJson)`,
`applyImageFragmentMove(bytes, requestJson, approvedPlanSha256)` and
`imageFragmentBindingsJson(bytes)` expose the native image-state capsule route.
The apply result has `bytes()`, `reportJson()` and `free()` methods. Run these
synchronous calls in a worker. Accepting their output requires explicitly
reopening any retained story session; they do not mutate its bytes or receipts.

The required `stack` is `foreground` or `background`. Original sources bind both
an occurrence ID and its `content_stream_index`; repeat moves use the returned
ownership binding. This is standalone image movement, not automatic linked
image/caption reflow. Tagged figures, initial nested-Form images, text-derived
clipping and page transparency groups require further migration. See
[`image_fragment_implementation.md`](../../docs/image_fragment_implementation.md).
No generated WASM, ABI or browser execution qualification is claimed.

### Fresh authored typed tables (source-only, unqualified)

`WellfriendPdf.authoredTypedTableSourcesJson()` reopens the private typed-table
registry and returns exact cell owners, logical source ranges and bounded cell
rectangles. `mutateAuthoredTypedTable(requestJson, fontBytes?)` applies a
revision-bound value/formula update, recalculates dependents and returns new PDF
bytes plus its validation report; it does not mutate the current object. Set
`prune_empty_continuations` in the request JSON to opt into guarded removal of
empty table-created continuation pages; original, modified or referenced pages
are retained and reported.

### Form XObject text (source-only, unqualified)

`WellfriendPdf.formTextSourcesJson(page)` returns direct text models with exact
revision/Contents-slot/invocation targets. `editFormText(requestJson, fontBytes?)`
accepts `{ target, edit, shared_form_policy }`, where `edit` uses the existing
multi-run logical-range schema. Choose `clone_edit_one_instance` or `edit_all_uses`
explicitly. Edit regions use Form-local coordinates before Matrix/caller
transforms; the inventory includes that Form's BBox.

The returned `WellfriendOutput` contains new bytes and an edit report with
`target_after`. Reopen accepted output before editing again. These synchronous
calls belong in a worker and do not mutate retained story sessions. Shared tagged
owners, external ActualText, cross-Form selections and appearance/pattern programs
remain boundaries. See [`form_text_implementation.md`](../../docs/form_text_implementation.md).
No compilation, generated WASM, binding or browser execution was performed.

### Revision-bound text paint partitions (source-only, unqualified)

`proposeTextRangePaintPartitions(requestJson)` returns the exact-revision
proposal JSON without mutating the current object. After reviewing one region
and optional final-line layout for every candidate, pass the original request,
proposal envelope and approval JSON to
`applyTextRangePaintPartitions(..., fontBytes?)`. Optional approved font bytes
drive shaping, measurement and PDF embedding when retained source coverage is
insufficient. Apply recomputes the canonical proposal and fails closed for
stale, altered, missing or reordered approvals. A supplied font requires its
lowercase SHA-256 in approval field `font_sha256`. The synchronous apply returns a new
`WellfriendOutput`; reopen accepted bytes explicitly and run it in a worker.
For governed publication, call `previewTextRangePaintPartitions`, show the PNG
evidence, and pass its `publication_receipt` to
`applyReviewedTextRangePaintPartitions`. The worker client retains this receipt
privately and uses the reviewed route automatically.
The WASM surface also exposes
`authenticateTextRangePaintPartitionReceipt` and
`verifyAuthenticatedTextRangePaintPartitionReceipt` for trusted local hosts.
They accept only non-negative JavaScript-safe integer timestamps. Never embed a
server-held HMAC key in browser-delivered code; browser clients should call the
authenticated HTTP preview/apply endpoints instead.
`browser/authenticated-paint-partition-client.js` implements that remote path
without accepting the HMAC key. Its TypeScript surface contains the canonical
request/proposal/approval/receipt models; it bounds inputs and responses,
checks exact envelope and receipt schemas, confirms the signed receipt wraps
the preview receipt, and parses the final PDF multipart response as bytes.

Every SDK-backed WASM document route whose native facade accepts an input
password now forwards the retained zeroizing credential, including editing,
reflow, standards, XFA, signatures, semantics, writer, redaction, associated
files and sanitation paths. It is not serialized into reports or reused for
output encryption.
