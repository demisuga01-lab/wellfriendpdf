# roadmap closure 02C Gradle Closure

Gradle Packaging closes the remaining Java build-surface caveat from Java Packaging.
Maven remains supported, and Gradle is now a real Java build/package surface.

| Row | Prior status | Gradle Packaging result | Evidence | Remaining limit | Blocks Release Packaging |
| --- | --- | --- | --- | --- | --- |
| Gradle build files | No authoritative Gradle build | Added `bindings/java/settings.gradle`, `bindings/java/build.gradle`, and `bindings/java/gradle.properties` | `scripts/gradle_packaging_gradle_package_smoke.ps1`; Gradle `clean test`, `jar`, and `build` | Requires JDK 25 preview FFM, same as Maven | no |
| Gradle bootstrap | Global Gradle unavailable | Script downloads pinned Gradle 9.6.1 to `target/gradle_packaging-tools` and verifies SHA-256 | `target/binding_parity-binding-parity/gradle-jar-smoke.json` records Gradle tool and commands | Network needed on hosts without global Gradle and without cached target tool | no |
| Gradle test path | Docs-only consumer guidance | Gradle `test` compiles test sources and runs `io.wellfriendpdf.WellfriendPdfSmokeTest` via JavaExec | `gradle --no-daemon -p bindings/java clean test`; Java smoke asserts reports, outputs, password-open, and progress/cancellation posture | No JUnit dependency; smoke remains a main-class test by design | no |
| Gradle JAR package | Missing | Gradle produces `bindings/java/build/libs/wellfriendpdf-sdk-0.1.0.jar` | JAR inspected by ZIP entries in `scripts/gradle_packaging_gradle_package_smoke.ps1` | Build-tool manifest fields differ from Maven | no |
| Gradle packaged runtime smoke | Missing | `PackageSmoke` runs from the Gradle JAR with `WELLFRIENDPDF_NATIVE_LIBRARY` unset | `target/binding_parity-binding-parity/gradle-jar-smoke.json`; native copied to `bindings/java/build/libs/runtimes/<rid>/native` | Platform proof is Windows x64 on this host; loader supports Linux/macOS names | no |
| Maven preservation | Maven passed in 02B | Maven smoke still runs through `scripts/java_packaging_java_package_smoke.ps1` | `target/binding_parity-binding-parity/java-package-smoke.json` | Uses target-local Maven fallback when host `mvn` is absent | no |
| Maven/Gradle equivalence | Not present | Public class list and reflection API summaries match; `Automatic-Module-Name` matches | `target/binding_parity-binding-parity/java-maven-gradle-equivalence.json` | Maven/Gradle generated manifest fields may differ intentionally | no |
| Gap matrix | Gradle row `unsupported_reported` | `java_packaging.java.gradle_policy` and `gradle_packaging.java.gradle_package` are `implemented_public` for Java/docs/packaging | `docs/bindings_binding_parity_gap_matrix.md`; `target/binding_parity-binding-parity/binding-gap-matrix.json` | None for Gradle Packaging | no |

Progress and cancellation remain honestly unsupported for Binding Parity binding
report/output surfaces. Gradle tests assert that feature-report status instead
of advertising fake callbacks or ignored tokens.
