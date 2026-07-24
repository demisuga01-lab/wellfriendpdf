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

## Prompt 02 Surface

Reports: feature, engine/ABI version, security, parser, color, validation,
forms, annotations, page operations, interactive content, legacy chunks,
Prompt 15 semantic bundles, advanced chunks, and provenance-aware search.

Outputs: sanitize, canonicalize, redact terms, DOCX, XLSX, PPTX, and Office to
PDF conversion helpers.

Password open is available through `WellfriendPdf.Document.open(path, password)` and
`WellfriendPdf.Document.open(bytes, password)`. Passwords are UTF-8 operation-scoped
inputs and are not retained on the Java document object.

Maven and Gradle are both package flows. `scripts/prompt02b_java_package_smoke.ps1`
runs Maven test/package, inspects `bindings/java/target/wellfriendpdf-sdk-0.1.0.jar`,
and runs a JAR-based package smoke. `scripts/prompt02c_gradle_package_smoke.ps1`
downloads pinned Gradle 9.6.1 when needed, runs Gradle `clean test`, `jar`, and
`build`, inspects `bindings/java/build/libs/wellfriendpdf-sdk-0.1.0.jar`, runs the same
JAR-based smoke from the Gradle artifact, and writes Maven/Gradle equivalence
evidence.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/prompt02b_java_package_smoke.ps1
powershell -ExecutionPolicy Bypass -File scripts/prompt02c_gradle_package_smoke.ps1
```

Known limits: progress and cancellation are reported through
`WellfriendPdf.featureReportJson()` as unsupported for the Prompt 02 binding surface; no
no-op callbacks or ignored interruption APIs are exposed. Mobile packaging is
out of scope for this binding.
