# Wellfriend Java Binding

## ECBES universal transaction

`Document.ecbesUniversalEdit(requestJson)` evaluates a bounded set of canonical
universal-edit candidates from one immutable revision and returns only the
evidence-qualified result. A governed no-selection returns the exact original
transport. Use `ecbesUniversalEditWithOutputCredentialBytes` for Standard-security
output candidates; credentials are apply-only and excluded from reports. See the
[ECBES specification](../../docs/research/evidence_constrained_bidirectional_edit_synthesis.md).

## Retained story editing

`WellfriendPdf.StoryEditSession` owns copied PDF bytes and exposes
`previewJson`, exact-receipt `checkpointJson`, `bytes`, `undo`, `redo`,
`commandJson` and explicitly requested `renderPagePng`.
Use/close the session on its creator thread. Optional `RenderCancellation`
arguments reuse the existing cooperative token; `cancel()` may be signalled
from a different thread. Close the token only after the operation returns.
The shared JSON command protocol exposes source inventories, table drafts and
conservative merge reports without introducing a separate editing engine.

The permissions/owner string-password constructor wipes its temporary UTF-8
bytes; `StoryEditSession.openWithPasswordBytes` accepts exact PDF password bytes.
User/open passwords are rejected. Encrypted input becomes an explicitly reported
unencrypted working revision; the session retains no credential and output
encryption remains explicit.

This addition targets the existing JDK 25 FFM binding on 64-bit platforms. The
Java package compiles against the new declarations and its focused story-session
suite passes against the refreshed local native DLL. Real encrypted-file and
cross-platform qualification remain pending.
See [protocol, ownership, limits and pending qualification](../../docs/native_story_sessions.md).

## Existing document binding

Dependency-free Java/JVM binding for Wellfriend using the Java Foreign Function &
Memory API. The verified local build uses JDK 25 on Windows x64. The binding
wraps the stable C ABI and preserves the shared versioned JSON report envelope.

```java
try (var doc = WellfriendPdf.Document.open(Path.of("report.pdf"), null)) {
    System.out.println(doc.pageCount());
    System.out.println(doc.page(1).text());
    System.out.println(doc.securityReportJson());
    System.out.println(doc.semanticBundleJson());
    System.out.println(doc.advancedChunksJson());
    System.out.println(doc.semanticSearchJson("invoice"));
    System.out.println(doc.authoredTypedTableSourcesJson());
    System.out.println(WellfriendPdf.codecIsolationReportJson(
        "FlateDecode",
        new byte[] {(byte) 0x78, (byte) 0x9c, (byte) 0xcb},
        "report_only"));

    WellfriendPdf.BinaryResult sanitized = doc.sanitize("balanced");
    Files.write(Path.of("sanitized.pdf"), sanitized.bytes());
    System.out.println(sanitized.reportJson());

    WellfriendPdf.BinaryResult tableEdit = doc.mutateAuthoredTypedTable(requestJson);
    Files.write(Path.of("table-edited.pdf"), tableEdit.bytes());

    Files.write(Path.of("report.docx"), doc.toDocx(true));
    Files.write(Path.of("from-word.pdf"), WellfriendPdf.Office.docxToPdf(doc.toDocx(true)));

    WellfriendPdf.RenderContract contract = doc.defaultRenderContract(1, 72)
        .withBackground(248, 250, 252)
        .withResourceBudget(20_000_000L, null, null, null);
    Files.write(Path.of("page.png"), doc.renderPagePng(contract));
    ByteBuffer surface = ByteBuffer.allocateDirect(Math.toIntExact(contract.surfaceByteLength()));
    doc.renderPageIntoBuffer(contract, surface);

    try (var renderCancellation = new WellfriendPdf.RenderCancellation()) {
        Files.write(Path.of("page-cancellable.png"), doc.renderPagePng(contract, renderCancellation));
        System.out.println(renderCancellation.isCancelled());
    }
}
```

## Native Loading

Set `WELLFRIENDPDF_NATIVE_LIBRARY` to the platform-specific `wellfriendpdf_capi` dynamic library
during development. The loader also checks the current directory,
`target/debug`, `target/release`, and `runtimes/<rid>/native` under both the
current directory and the JAR/package directory.

Run the smoke test directly:

```powershell
javac --enable-preview --release 25 -d bindings/java/target/classes `
  (Get-ChildItem bindings/java/src/main/java -Recurse -Filter *.java).FullName `
  (Get-ChildItem bindings/java/src/test/java -Recurse -Filter *.java).FullName
java --enable-preview --enable-native-access=ALL-UNNAMED `
  -cp bindings/java/target/classes io.wellfriendpdf.WellfriendPdfSmokeTest
```

For a source-only render-contract builder check that does not load the native
library, run the smoke main with `--contract-builder-only`.

## Binding Parity Surface

Reports: feature, engine/ABI version, security, parser, color, validation,
forms, annotations, page operations, interactive content, legacy chunks,
Semantic Closeout semantic bundles, advanced chunks, and provenance-aware search.

Outputs: sanitize, canonicalize, redact terms, DOCX, XLSX, PPTX, and Office to
PDF conversion helpers.

Renderer contract APIs expose the schema-v1 native contract as a typed
`WellfriendPdf.RenderContract` object. Callers can round-trip default contract
JSON, set surface layout, background, clip, transform, page box,
optional-content identity, annotation/form policy, execution/backend/compositing
policy, smoothing, prepress/color policy, exactness, determinism, and resource
budgets, and then pass the typed contract to PNG or caller-owned buffer render
methods.

Contract rendering exposes cooperative cancellation through
`WellfriendPdf.RenderCancellation`, including `isCancelled()`, for PNG,
caller-owned direct-buffer, font-substitution report, and render-telemetry
report methods. Existing compatibility overloads remain non-cancellable.

Editing transaction APIs include
`editing_transactionsTransactionApplyWithRenderInvalidation(requestJson,
renderInvalidationOptionsJson)`, which returns edited PDF bytes plus the shared
SDK JSON report containing mapped render source IDs and optional dirty render
tiles for caller cache invalidation.

Caller-owned render caches are exposed through `WellfriendPdf.RenderCache`.
Documents can render contract PNGs through the cache, request cache telemetry
with `renderPagePngWithRenderCacheReport`, and apply the SDK/server
`report.render_invalidation` JSON through
`RenderCache.applyRenderInvalidationPlanJson`.

Image decode APIs include `imageDecodeCapabilityReportJson()` and
`progressiveImageDecodeLifecycleReportJson(requestJson)`, exposing per-image
region/reduction/progressive capability status and bounded start/continue/pause/
resume/cancel/fail/close/document_close lifecycle reports without decoding
pixels.

Progressive sessions expose `reviseRenderContract` and
`reviseRenderContractJson` for full live schema-v1 contract revision, explicit
request-cancel, viewport and dirty-region revision JSON, tile-publication
evaluation JSON, viewer queue preview JSON,
`executeViewerQueueJson`, `executeAdjacentPagePrefetch`, viewer callback
dispatch JSON, `dispatchViewerCallbacks`, `BooleanSupplier` cancellation
overloads for step/finish calls, and `RenderCancellation` overloads for
viewer-queue execution and adjacent-page prefetch execution.

Password open is available through `WellfriendPdf.Document.open(path, password)` and
`WellfriendPdf.Document.open(bytes, password)`. Exact byte credentials use
`openWithPasswordBytes`. String conveniences wipe their temporary UTF-8 arrays;
the native document handle retains a private zeroizing credential copy only so
revision-bound operations can reparse the immutable encrypted source. It is not
serialized or reused as an output password.

Maven and Gradle are both package flows. `scripts/java_packaging_java_package_smoke.ps1`
runs Maven test/package, inspects `bindings/java/target/wellfriendpdf-sdk-0.1.0.jar`,
and runs a JAR-based package smoke. `scripts/gradle_packaging_gradle_package_smoke.ps1`
downloads pinned Gradle 9.6.1 when needed, runs Gradle `clean test`, `jar`, and
`build`, inspects `bindings/java/build/libs/wellfriendpdf-sdk-0.1.0.jar`, runs the same
JAR-based smoke from the Gradle artifact, and writes Maven/Gradle equivalence
evidence.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/java_packaging_java_package_smoke.ps1
powershell -ExecutionPolicy Bypass -File scripts/gradle_packaging_gradle_package_smoke.ps1
```

Known limits: external native binding runtime matrices, viewer runtime matrices,
cross-language queue policy validation, and Java interruption/future adapters
are deferred verification or optional host-adapter work. Contract render
cancellation and the complete progressive state machine are source-visible.
Mobile packaging is out of scope for this binding.

### Revision-bound text paint partitions (source-only, unqualified)

`Document.proposeTextRangePaintPartitions(requestJson)` returns a non-mutating
proposal bound to the exact document and complete request. After reviewing one
region and optional final-line layout per candidate, call
`applyTextRangePaintPartitions(requestJson, proposalJson, approvalJson,
fontBytes?)`. Optional approved font bytes drive shaping, measurement and PDF
embedding when the retained source program lacks coverage. Application
recomputes the canonical proposal and rejects stale or altered identities and
incomplete/reordered approvals. When `fontBytes` is supplied, the approval must
include its lowercase SHA-256 as `font_sha256`. This Java surface is source-wired but has not
been compiled or executed in the current source-only phase.
Interactive hosts can call `previewTextRangePaintPartitions`, display its PNG
evidence, then pass `report.publication_receipt` to
`applyReviewedTextRangePaintPartitions`; native code withholds mismatched output.
Trusted native hosts may use the static
`authenticateTextRangePaintPartitionReceipt` and
`verifyAuthenticatedTextRangePaintPartitionReceipt` methods with a
32..=256-byte HMAC-SHA-256 key. Never ship a server-held key to an untrusted
client; remote applications should use the authenticated HTTP routes.
Typed overloads are also available through `PaintPartitionTextRangeRequest`,
`PaintPartitionTextOptions`, `PaintPartitionProposal`,
`PaintPartitionApproval`, `PaintPartitionPreview`, and the typed receipt
wrappers. They validate geometry, scalar ranges, digests, receipt schemas and
SDK envelope kinds while preserving the existing raw-JSON overloads. The
binding's internal JSON codec now preserves finite decimal coordinates instead
of truncating them to integers.

Password-opened handles forward their retained native input credential through
every document-dependent C SDK facade route, including security, rendering,
parser, colour, standards, forms, editing, reflow, XFA, semantic, redaction and
sanitation paths. It is never serialized or silently promoted to an output
credential. The remaining text-only mapping/shaping/subsetting/substitution
utilities do not reopen or parse the document.
