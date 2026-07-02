# Java Binding

The Java binding lives under `bindings/java`. It uses the Java Foreign Function
& Memory API to call the stable C ABI directly.

Public shape:

- `Oxide.Document.open(path | bytes)`
- `doc.pageCount()`
- `doc.page(1).text()`
- `doc.parseJson()`
- `doc.toDocx(true)`, `doc.toXlsx("pages")`, `doc.toPptx(true)`
- `Oxide.Office.docxToPdf(...)`, `xlsxToPdf(...)`, `pptxToPdf(...)`

Documents implement `AutoCloseable`, so callers should use try-with-resources.

```java
try (var doc = Oxide.Document.open(Path.of("report.pdf"))) {
    System.out.println(doc.page(1).text());
    Files.write(Path.of("report.docx"), doc.toDocx(true));
    Files.write(Path.of("report.pdf"), Oxide.Office.docxToPdf(doc.toDocx(true)));
}
```

Development test command used on this host:

```powershell
$env:OXIDE_NATIVE_LIBRARY="E:\wellpdfsdk\target\debug\oxide_capi.dll"
$env:OXIDE_FIXTURE_PDF="E:\wellpdfsdk\crates\engine\tests\fixtures\tracemonkey.pdf"
javac --release 25 -d bindings/java/target/classes (Get-ChildItem bindings/java/src/main/java,bindings/java/src/test/java -Recurse -Filter *.java).FullName
java --enable-native-access=ALL-UNNAMED -cp bindings/java/target/classes org.oxidepdf.OxideSmokeTest
```

Verified here: Windows x64, JDK 25.0.2. Older JDKs would need a JNI/JNA adapter
or a separate compatibility package. Mobile packaging is future work.
