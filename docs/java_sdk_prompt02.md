# Java SDK Prompt 02

The Java SDK lives under `bindings/java` and uses the Java Foreign Function &
Memory API to call the stable C ABI. It is dependency-free and verified with
JDK 25 preview FFM on Windows x64.

## Package Shape

- Source: `bindings/java/src/main/java/org/oxidepdf/Oxide.java`
- Smoke test: `bindings/java/src/test/java/org/oxidepdf/OxideSmokeTest.java`
- Example: `bindings/java/examples/Prompt02Reports.java`
- Maven metadata: `bindings/java/pom.xml`

## Public API

Lifecycle:

- `Oxide.Document.open(Path)`
- `Oxide.Document.open(byte[])`
- `close()` via `AutoCloseable`
- `pageCount`, `page(n)`, `pages`, `extractText`

Reports:

- `Oxide.featureReportJson()`
- `Oxide.engineVersion()`, `Oxide.abiVersion()`
- `securityReportJson`
- `parserReportJson`
- `colorReportJson`
- `validateJson`
- `formsReportJson`
- `annotationsReportJson`
- `pagesReportJson`
- `interactiveReportJson`
- `chunksJson`

Outputs:

- `sanitize`
- `canonicalize`
- `redactTerms`
- `toDocx`, `toXlsx`, `toPptx`
- `Oxide.Office.docxToPdf`, `xlsxToPdf`, `pptxToPdf`

## Native Loading and Ownership

The FFM loader checks `OXIDE_NATIVE_LIBRARY`, the current directory, local
`target/debug` and `target/release`, and `runtimes/<rid>/native`. Native output
buffers are copied into Java `byte[]` values and released with
`oxide_buffer_free`. Native strings are released with `oxide_string_free` or
`oxide_error_free`. C ABI status and error text are preserved in
`OxideException`.

## Limits

Password open is not yet public because the C ABI open function accepts bytes
only. Progress and cancellation are not exposed because the current engine
facade calls do not observe binding-level cancellation. Maven packaging is
metadata-ready; the FFM preview requirement means CI must run with a matching
JDK and `--enable-preview --enable-native-access=ALL-UNNAMED`.
