# Oxide Java Binding

Dependency-free Java/JVM binding for Oxide using the Java Foreign Function &
Memory API. The verified local build uses JDK 25 on Windows x64.

```java
try (var doc = Oxide.Document.open(Path.of("report.pdf"))) {
    System.out.println(doc.pageCount());
    System.out.println(doc.page(1).text());
    Files.write(Path.of("report.docx"), doc.toDocx(true));
    Files.write(Path.of("from-word.pdf"), Oxide.Office.docxToPdf(doc.toDocx(true)));
}
```

Set `OXIDE_NATIVE_LIBRARY` to the platform-specific `oxide_capi` dynamic
library during development. Run examples/tests with:

```powershell
javac --enable-preview --release 25 -d bindings/java/target/classes (Get-ChildItem bindings/java/src/main/java -Recurse -Filter *.java).FullName
java --enable-preview --enable-native-access=ALL-UNNAMED -cp bindings/java/target/classes org.oxidepdf.OxideSmokeTest
```

Mobile packaging is deliberately out of scope for this binding.
