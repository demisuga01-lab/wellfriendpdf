# Wellfriend Java Binding

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
    System.out.println(WellfriendPdf.codecIsolationReportJson(
        "FlateDecode",
        new byte[] {(byte) 0x78, (byte) 0x9c, (byte) 0xcb},
        "report_only"));

    WellfriendPdf.BinaryResult sanitized = doc.sanitize("balanced");
    Files.write(Path.of("sanitized.pdf"), sanitized.bytes());
    System.out.println(sanitized.reportJson());

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
`WellfriendPdf.Document.open(bytes, password)`. Passwords are UTF-8 operation-scoped
inputs and are not retained on the Java document object.

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
