# Java SDK Prompt 02

The Java SDK lives under `bindings/java` and uses the Java Foreign Function &
Memory API to call the stable C ABI. It is dependency-free and verified with
JDK 25 preview FFM on Windows x64.

## Package Shape

- Source: `bindings/java/src/main/java/org/oxidepdf/Oxide.java`
- Smoke test: `bindings/java/src/test/java/org/oxidepdf/OxideSmokeTest.java`
- Example: `bindings/java/examples/Prompt02Reports.java`
- Maven metadata: `bindings/java/pom.xml`
- Gradle metadata: `bindings/java/settings.gradle`, `bindings/java/build.gradle`,
  `bindings/java/gradle.properties`

## Public API

Lifecycle:

- `Oxide.Document.open(Path)`
- `Oxide.Document.open(byte[])`
- `Oxide.Document.open(Path, String password)`
- `Oxide.Document.open(byte[], String password)`
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
`target/debug` and `target/release`, and `runtimes/<rid>/native` under both the
current directory and the JAR/package directory. Native output buffers are copied into Java `byte[]` values and released with
`oxide_buffer_free`. Native strings are released with `oxide_string_free` or
`oxide_error_free`. C ABI status and error text are preserved in
`OxideException`.

## Limits

Prompt 02B added password-aware open overloads. Passwords are encoded as UTF-8
for the native open call, are not logged, and are not retained on the Java
document object. C ABI tests cover correct/wrong passwords on a generated
encrypted fixture; Java tests cover the public overloads and verify malformed
input errors do not echo the supplied password.

Maven and Gradle are both real Java package surfaces.

Maven: `bindings/java/pom.xml` compiles with JDK 25 preview FFM flags, binds
`OxideSmokeTest` into `mvn test`, and produces
`bindings/java/target/oxide-sdk-0.1.0.jar`. The Prompt 02B package script
(`scripts/prompt02b_java_package_smoke.ps1`) downloads Maven 3.9.9 into
`target/prompt02b-tools` if no host `mvn` exists, runs `mvn clean test` and
`mvn package`, inspects the JAR, and runs `PackageSmoke` from the packaged JAR
with native loading through `runtimes/<rid>/native`.

Gradle: `bindings/java/build.gradle` uses the same JDK 25 preview FFM policy,
runs the existing `OxideSmokeTest` from the Gradle `test` task, and produces
`bindings/java/build/libs/oxide-sdk-0.1.0.jar`. The Prompt 02C package script
(`scripts/prompt02c_gradle_package_smoke.ps1`) downloads pinned Gradle 9.6.1
into `target/prompt02c-tools` if no host `gradle` exists, verifies the archive
checksum, runs Gradle `clean test`, `jar`, and `build`, inspects the Gradle JAR,
runs `PackageSmoke` from the Gradle-built artifact with native loading through
`build/libs/runtimes/<rid>/native`, and writes Maven/Gradle equivalence evidence.

Direct Gradle commands:

```powershell
gradle --no-daemon -p bindings/java clean test
gradle --no-daemon -p bindings/java jar
gradle --no-daemon -p bindings/java build
```

The Gradle `test` task uses a test-scope JUnit wrapper that invokes the same
dependency-free `OxideSmokeTest` main class used by the direct Java smoke path.

Maven and Gradle JARs are not required to be byte-identical because build-tool
manifest fields can differ. Prompt 02C requires public class/API equivalence,
matching `Automatic-Module-Name`, clean package contents, and successful runtime
smokes for both artifacts.

Progress and cancellation are not exposed because the current Prompt 02
report/output facade calls do not observe binding-level tokens. Query
`Oxide.featureReportJson()` for the machine-readable
`progress_not_supported` and
`cancellation_not_supported_for_prompt02_bindings` statuses.
