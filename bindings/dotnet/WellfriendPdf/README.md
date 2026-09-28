# WellfriendPdf

## ECBES universal transaction

`EcbesUniversalEdit(requestJson)` materializes the bounded canonical
universal-edit candidates declared in the request from one immutable revision and
returns only the evidence-qualified selection as `WellfriendBinaryResult`. If no
candidate qualifies, the returned bytes are the exact original transport. Use
`EcbesUniversalEditWithOutputCredentialBytes` when a candidate requests Standard
security; those binary-safe credentials are apply-only and never enter the report.
See the [ECBES specification](../../../docs/research/evidence_constrained_bidirectional_edit_synthesis.md)
for the request schema, fidelity classes, proof obligations, and current limits.

## Retained story editing

`StoryEditSession.Open(bytes)` retains the current PDF and provides
`PreviewJson(requestJson)`, `CheckpointJson(requestJson, receiptJson)`,
`Bytes()`, `Undo()`, `Redo()`, and explicitly requested `RenderPagePng()`.
Pass the exact `receipt` object from the preview response when checkpointing.
`CommandJson` exposes source inventories, table drafts and merge reports through
the shared engine protocol. Mutating calls support `CancellationToken`.
Calls and disposal are serialized; results are owned managed copies.

`StoryEditSession.Open(bytes, password)` accepts a permissions/owner credential
and wipes its temporary UTF-8 bytes. `OpenWithPasswordBytes` preserves exact PDF
password bytes. User/open passwords are rejected. An encrypted input becomes an
explicitly reported unencrypted working revision; credentials are not retained
and encrypted output is a separate, explicit publication choice.

The managed package builds against the new native declarations, and both focused
story-session tests pass against the refreshed local native DLL. Real
encrypted-file and cross-platform binding qualification remains pending.
See [protocol, ownership, limits and pending qualification](../../../docs/native_story_sessions.md).

## Existing document binding

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
Console.WriteLine(doc.AuthoredTypedTableSourcesJson());
Console.WriteLine(WellfriendDocument.CodecIsolationReportJson(
    "FlateDecode",
    Convert.FromHexString("789ccb48cdc9c957c8afc84c49050019dd044e"),
    "in_process"));

var sanitized = doc.Sanitize("balanced");
File.WriteAllBytes("sanitized.pdf", sanitized.Bytes);
Console.WriteLine(sanitized.ReportJson);

// Request JSON binds input_sha256, table_id and exact typed cell updates.
var tableEdit = doc.MutateAuthoredTypedTable(requestJson);
File.WriteAllBytes("table-edited.pdf", tableEdit.Bytes);

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

### Revision-bound text paint partitions (source-only, unqualified)

`ProposeTextRangePaintPartitions(requestJson)` returns a non-mutating proposal
whose input hash, request hash, proposal ID, source paint slots and complete
grapheme-safe replacement ranges are fixed. The caller reviews one bounded
region and optional final-line layout for every candidate, then passes the
original request, proposal envelope and approval JSON to
`ApplyTextRangePaintPartitions(..., fontBytes?)`. Optional approved font bytes
drive shaping/measurement/PDF embedding when the retained source program lacks
coverage. Apply recomputes the proposal from the current document and rejects
stale, altered, missing or reordered approvals. When `fontBytes` is supplied,
the approval must include its lowercase SHA-256 as `font_sha256`.
Interactive hosts should instead call `PreviewTextRangePaintPartitions`, show
the returned PNG evidence, retain `report.publication_receipt`, and pass it to
`ApplyReviewedTextRangePaintPartitions`. Native code withholds output if any
bound input, decision, font, or candidate digest differs.
Trusted native hosts may wrap that content receipt with
`AuthenticateTextRangePaintPartitionReceipt` and verify it with
`VerifyAuthenticatedTextRangePaintPartitionReceipt`. These helpers borrow a
32..=256-byte HMAC-SHA-256 key; never distribute a server key to an untrusted
client. Remote clients should use the authenticated HTTP preview/apply routes.
The same workflow now has strongly typed overloads built from
`PaintPartitionTextRangeRequest`, `PaintPartitionTextEditOptions`,
`PaintPartitionProposal`, `PaintPartitionApproval`, the two receipt records,
and `PaintPartitionEnvelope<T>`. These models serialize the canonical
snake-case wire schema, validate regions/ranges/digests before native entry,
reject an unexpected envelope kind on return, and leave the original JSON
overloads intact for forward-compatible or low-level callers.
This managed path has not yet been compiled or executed in the current
source-only phase.

Password-opened handles forward their retained native input credential through
every document-dependent C SDK facade route, including security, rendering,
parser, colour, standards, forms, editing, reflow, XFA, semantic, redaction and
sanitation paths. The credential is not serialized into plans/reports or reused
for output encryption. The remaining text-only mapping/shaping/subsetting/
substitution utilities do not reopen or parse the document.
