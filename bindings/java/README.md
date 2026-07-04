# Oxide Java Binding

Dependency-free Java/JVM binding for Oxide using the Java Foreign Function &
Memory API. The verified local build uses JDK 25 on Windows x64. The binding
wraps the stable C ABI and preserves the shared versioned JSON report envelope.

```java
try (var doc = Oxide.Document.open(Path.of("report.pdf"))) {
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
`target/debug`, `target/release`, and `runtimes/<rid>/native`.

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

Known limits: password open is not yet exposed through the C ABI-backed Java
surface; progress and cancellation are not exposed because current engine calls
do not observe binding-level tokens. Mobile packaging is out of scope for this
binding.
