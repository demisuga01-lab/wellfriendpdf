# WellfriendPdf

Idiomatic .NET binding for Wellfriend's Rust PDF engine. The package wraps the
stable C ABI with P/Invoke and returns the same versioned JSON report envelopes
as Rust, Python, C ABI, WASM, and Java.

```csharp
using WellfriendPdf;

using var doc = WellfriendDocument.Open("report.pdf", password: null);
Console.WriteLine(doc.PageCount);
Console.WriteLine(doc.GetPage(1).Text);
Console.WriteLine(doc.SecurityReportJson());
Console.WriteLine(doc.SemanticBundleJson());
Console.WriteLine(doc.AdvancedChunksJson());
Console.WriteLine(doc.SemanticSearchJson("invoice"));
Console.WriteLine(WellfriendDocument.CodecIsolationReportJson(
    "FlateDecode",
    Convert.FromHexString("789ccb48cdc9c957c8afc84c49050019dd044e"),
    "in_process"));

var sanitized = doc.Sanitize("balanced");
File.WriteAllBytes("sanitized.pdf", sanitized.Bytes);
Console.WriteLine(sanitized.ReportJson);

File.WriteAllBytes("report.docx", doc.ToDocx());
File.WriteAllBytes("report.xlsx", doc.ToXlsx(layout: "pages"));
File.WriteAllBytes("report.pptx", doc.ToPptx());
File.WriteAllBytes("from-word.pdf", OfficeConverters.DocxToPdf("report.docx"));

var contract = doc.DefaultRenderContract(pageNumber: 1)
    .WithBackground(248, 250, 252)
    .WithResourceBudget(maxPixels: 20_000_000);
File.WriteAllBytes("page.png", doc.RenderPagePng(contract));
var surface = new byte[checked((int)contract.SurfaceByteLength)];
doc.RenderPageIntoBuffer(contract, surface);

using var renderCancellation = new RenderCancellation();
File.WriteAllBytes("page-cancellable.png", doc.RenderPagePng(contract, renderCancellation));
Console.WriteLine(renderCancellation.IsCancelled);
```

## Native Loading

During development, set `WELLFRIENDPDF_NATIVE_LIBRARY` to the platform-specific
`wellfriendpdf_capi` dynamic library. When packaged, the resolver also checks:

- `AppContext.BaseDirectory`
- the assembly directory
- the current directory
- `target/debug` and `target/release`
- `runtimes/<rid>/native`

Use `using` or `Dispose()` for documents. Native handles are owned by
`SafeHandle`; output buffers are copied into managed `byte[]` values and freed
before methods return.

## Binding Parity Surface

Reports: feature, engine/ABI version, security, parser, color, validation,
forms, annotations, page operations, interactive content, legacy chunks,
Semantic Closeout semantic bundles, advanced chunks, and provenance-aware search.

Outputs: sanitize, canonicalize, redact terms, DOCX, XLSX, PPTX, and Office to
PDF conversion helpers.

Renderer contract APIs expose the schema-v1 native contract as a managed
`RenderContract` object. Callers can round-trip the default JSON, set surface
layout, background, clip, transform, page box, optional-content identity,
annotation/form policy, execution/backend/compositing policy, smoothing,
prepress/color policy, exactness, determinism, and resource budgets, then pass
the typed contract to PNG or caller-owned buffer render methods.

Contract rendering exposes cooperative cancellation through
`RenderCancellation`, including `IsCancelled`, and through `CancellationToken`
overloads for PNG, caller-owned buffers, font-substitution report renders, and
render-telemetry report renders. The cancellation source is additive; existing
compatibility overloads remain non-cancellable.

Editing transaction APIs include
`EditingTransactionsTransactionApplyWithRenderInvalidation(requestJson,
renderInvalidationOptionsJson)`, which returns edited PDF bytes plus the shared
SDK JSON report containing mapped render source IDs and optional dirty render
tiles for caller cache invalidation.

Caller-owned render caches are exposed through `RenderCache`. A document can
render contract PNGs through the cache, request cache telemetry with
`RenderPagePngWithRenderCacheReport`, and apply the SDK/server
`report.render_invalidation` JSON through
`RenderCache.ApplyRenderInvalidationPlanJson`.

Image decode APIs include `ImageDecodeCapabilityReportJson()` and
`ProgressiveImageDecodeLifecycleReportJson(requestJson)`, exposing per-image
region/reduction/progressive capability status and bounded start/continue/pause/
resume/cancel/fail/close/document_close lifecycle reports without decoding
pixels.

Password open is available through `WellfriendDocument.Open(path, password)` and
`WellfriendDocument.Open(bytes, password)`. Passwords are UTF-8 operation-scoped
inputs and are not retained on the managed document object.

Progressive sessions expose `ReviseRenderContract` and
`ReviseRenderContractJson` for full live schema-v1 contract revision, explicit
request-cancel, viewport revision JSON, dirty-region revision JSON,
tile-publication evaluation JSON,
viewer queue preview JSON, `ExecuteViewerQueueJson`,
`ExecuteAdjacentPagePrefetch`, viewer callback dispatch JSON,
`DispatchViewerCallbacks`, terminal cancel methods, and `CancellationToken`
overloads for step, finish, viewer-queue execution, and adjacent-page prefetch
execution. Contract render cancellation is source-visible; external native
binding runtime matrices, viewer runtime matrices, and broader cross-language
queue policy validation are deferred verification work.
Mobile packaging is out of scope for this package.
