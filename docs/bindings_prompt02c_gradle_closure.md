# Combined Prompt 02C Gradle Closure

Prompt 02C closes the remaining Java build-surface caveat from Prompt 02B.
Maven remains supported, and Gradle is now a real Java build/package surface.

| Row | Prior status | Prompt 02C result | Evidence | Remaining limit | Blocks Prompt 03 |
| --- | --- | --- | --- | --- | --- |
| Gradle build files | No authoritative Gradle build | Added `bindings/java/settings.gradle`, `bindings/java/build.gradle`, and `bindings/java/gradle.properties` | `scripts/prompt02c_gradle_package_smoke.ps1`; Gradle `clean test`, `jar`, and `build` | Requires JDK 25 preview FFM, same as Maven | no |
| Gradle bootstrap | Global Gradle unavailable | Script downloads pinned Gradle 9.6.1 to `target/prompt02c-tools` and verifies SHA-256 | `target/prompt02-binding-parity/gradle-jar-smoke.json` records Gradle tool and commands | Network needed on hosts without global Gradle and without cached target tool | no |
| Gradle test path | Docs-only consumer guidance | Gradle `test` compiles test sources and runs `io.wellfriendpdf.WellfriendPdfSmokeTest` via JavaExec | `gradle --no-daemon -p bindings/java clean test`; Java smoke asserts reports, outputs, password-open, and progress/cancellation posture | No JUnit dependency; smoke remains a main-class test by design | no |
| Gradle JAR package | Missing | Gradle produces `bindings/java/build/libs/wellfriendpdf-sdk-0.1.0.jar` | JAR inspected by ZIP entries in `scripts/prompt02c_gradle_package_smoke.ps1` | Build-tool manifest fields differ from Maven | no |
| Gradle packaged runtime smoke | Missing | `PackageSmoke` runs from the Gradle JAR with `WELLFRIENDPDF_NATIVE_LIBRARY` unset | `target/prompt02-binding-parity/gradle-jar-smoke.json`; native copied to `bindings/java/build/libs/runtimes/<rid>/native` | Platform proof is Windows x64 on this host; loader supports Linux/macOS names | no |
| Maven preservation | Maven passed in 02B | Maven smoke still runs through `scripts/prompt02b_java_package_smoke.ps1` | `target/prompt02-binding-parity/java-package-smoke.json` | Uses target-local Maven fallback when host `mvn` is absent | no |
| Maven/Gradle equivalence | Not present | Public class list and reflection API summaries match; `Automatic-Module-Name` matches | `target/prompt02-binding-parity/java-maven-gradle-equivalence.json` | Maven/Gradle generated manifest fields may differ intentionally | no |
| Gap matrix | Gradle row `unsupported_reported` | `prompt02b.java.gradle_policy` and `prompt02c.java.gradle_package` are `implemented_public` for Java/docs/packaging | `docs/bindings_prompt02_gap_matrix.md`; `target/prompt02-binding-parity/binding-gap-matrix.json` | None for Prompt 02C | no |

Progress and cancellation remain honestly unsupported for Prompt 02 binding
report/output surfaces. Gradle tests assert that feature-report status instead
of advertising fake callbacks or ignored tokens.
