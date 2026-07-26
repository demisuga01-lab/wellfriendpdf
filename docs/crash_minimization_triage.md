# Crash minimization and triage

`scripts/minimize_prompt29_failures.py` builds the Prompt 29 crash, hang, OOM, unified failure, minimization, bug-triage, and fixed-regression artifacts.

The triage model covers:

- Prompt 27 parser fuzz artifacts;
- Prompt 28 codec/renderer/writer/SafeDocs artifacts;
- Prompt 29 malformed corpus and differential findings;
- sanitizer failures;
- newly minimized reproducers and promoted regression seeds.

Each finding records source campaign, target or corpus file, command, exit code or signal, sanitizer class, sanitized panic/source location if known, artifact path, SHA-256, root cause, severity, fix status, regression test path, rerun status, and future owner.

Raw artifacts stay under ignored result directories. Committed reports contain sanitized metadata and SHA-256 hashes only.

Closure requires zero unclassified Prompt 29-owned crash, hang, OOM, sanitizer, false-valid, redaction leak, signature trust falsehood, or unbounded-allocation finding.
