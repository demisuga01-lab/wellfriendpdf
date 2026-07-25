# Prompt 27 parser fuzz crash triage

Crash artifacts are kept under ignored Prompt 27 result directories and are not committed
unless they are minimized, legal, compact, and useful as regression seeds.

Triage steps:

1. Save the raw artifact and record target, command, toolchain, memory cap, timeout, and
   SHA-256.
2. Run `scripts/minimize_fuzz_crash.py <target> <artifact>` where cargo-fuzz minimization
   is applicable.
3. Classify the result as parser bug, expected unsupported input, timeout, OOM,
   sanitizer-only issue, duplicate, false positive, or external/toolchain failure.
4. Add a focused regression test or seed for valid bugs.
5. Fix the parser or record the exact blocker.
6. Rerun the minimized case and affected parser tests.

Any unclassified crash, hang, timeout, or OOM blocks Prompt 27 closure.
