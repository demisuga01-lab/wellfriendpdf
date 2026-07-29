# Prompt 35 Deferred Validation

Prompt 35 uses the same implementation-first posture as Prompt 34. The minimum
gate verifies formatting, diff hygiene, workspace compilation, focused runtime
tests, save/reopen smoke, touched binding compile checks, and one public-surface
runtime smoke.

The following are not marked as passed by Prompt 35:

- long fuzzing;
- large malformed-input campaigns;
- large public-corpus downloads;
- exhaustive accessibility corpora;
- full standards/profile validation;
- full differential rendering;
- viewer compatibility matrices;
- full binding package release matrix;
- performance and stress benchmarks;
- historical gate replay.

These are explicitly deferred to Prompt 36.

The Prompt 35 minimum gate compiles Java through Maven. The VPS Gradle
installation is older than the repository settings model and is classified as a
host-tooling incompatibility for Prompt 35, not as a passed Gradle package
matrix. Full Gradle package validation remains part of Prompt 36.
