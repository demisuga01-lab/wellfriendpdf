# document security Deferred Validation

document security uses the same implementation-first posture as document subsystems. The minimum
gate verifies formatting, diff hygiene, workspace compilation, focused runtime
tests, save/reopen smoke, touched binding compile checks, and one public-surface
runtime smoke.

The following are not marked as passed by document security:

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

These are explicitly deferred to release validation.

The document security minimum gate compiles Java through Maven. The VPS Gradle
installation is older than the repository settings model and is classified as a
host-tooling incompatibility for document security, not as a passed Gradle package
matrix. Full Gradle package validation remains part of release validation.
