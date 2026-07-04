# Oxide Java Binding

Dependency-free Java/JVM binding for Oxide using the Java Foreign Function &
Memory API. The verified local build uses JDK 25 on Windows x64. The binding
wraps the stable C ABI and preserves the shared versioned JSON report envelope.

```java
try (var doc = Oxide.Document.open(Path.of("report.pdf"), null)) {
    System.out.println(doc.pageCount());
    System.out.println(doc.page(1).text());
    System.out.println(doc.securityReportJson());

    Oxide.BinaryResult sanitized = doc.sanitize("balanced");
    Files.write(Path.of("sanitized.pdf"), sanitized.bytes());
    System.out.println(sanitized.reportJson());

    Files.write(Path.of("report.docx"), doc.toDocx(true));
    Files.write(Path.of("from-word.pdf"), Oxide.Office.docxToPdf(doc.toDocx(true)));
}
```

## Native Loading

Set `OXIDE_NATIVE_LIBRARY` to the platform-specific `oxide_capi` dynamic library
during development. The loader also checks the current directory,
`target/debug`, `target/release`, and `runtimes/<rid>/native` under both the
current directory and the JAR/package directory.

Run the smoke test directly:

```powershell
javac --enable-preview --release 25 -d bindings/java/target/classes `
  (Get-ChildItem bindings/java/src/main/java -Recurse -Filter *.java).FullName `
  (Get-ChildItem bindings/java/src/test/java -Recurse -Filter *.java).FullName
java --enable-preview --enable-native-access=ALL-UNNAMED `
  -cp bindings/java/target/classes org.oxidepdf.OxideSmokeTest
```

## Prompt 02 Surface

Reports: feature, engine/ABI version, security, parser, color, validation,
forms, annotations, page operations, interactive content, and chunks.

Outputs: sanitize, canonicalize, redact terms, DOCX, XLSX, PPTX, and Office to
PDF conversion helpers.

Password open is available through `Oxide.Document.open(path, password)` and
`Oxide.Document.open(bytes, password)`. Passwords are UTF-8 operation-scoped
inputs and are not retained on the Java document object.

Maven is the authoritative package flow. `scripts/prompt02b_java_package_smoke.ps1`
runs Maven test/package, inspects `bindings/java/target/oxide-sdk-0.1.0.jar`,
and runs a JAR-based package smoke. Gradle is documented as consumer-only over
the Maven/JAR artifact rather than a second build system.

Known limits: progress and cancellation are reported through
`Oxide.featureReportJson()` as unsupported for the Prompt 02 binding surface; no
no-op callbacks or ignored interruption APIs are exposed. Mobile packaging is
out of scope for this binding.
