# Prompt 23B Fuzzing Evidence

This document records the final Prompt 23B bounded fuzz-smoke posture. It does
not claim release-duration fuzzing.

## Harness Change

`scripts/ci_fuzz.py` now records target progress and writes a machine-readable
JSON report. Each cargo-fuzz phase has a bounded timeout, target index, start
time, configured fuzz duration, max input length, process ID, exit status,
elapsed time, corpus count, artifact count, and process-tree cleanup result.

The harness also enforces a global campaign deadline by capping each build or
run phase to the remaining global time. On timeout it terminates the child
process tree and reports the target and phase as failed; forced termination is
never counted as success.

## Timeout Root Cause

The earlier command:

```powershell
python scripts/ci_fuzz.py --targets crypto --mode smoke --seconds 5 --max-len 4096
```

timed out because the cold `cargo +nightly fuzz build crypto` phase took longer
than the previous outer five-minute command limit. The fuzz target itself did
not hang: after the build completed, the `crypto` smoke run exited successfully
in under ten seconds.

## Final Bounded Smoke

Final bounded command:

```powershell
python scripts/ci_fuzz.py --targets crypto --mode smoke --seconds 5 --max-len 4096 --json-report target/prompt23-writer-crypto/prompt23b-fuzz-smoke-results.json --global-timeout 1200 --per-target-timeout 75 --build-timeout 900 --kill-grace 5
```

Result:

- target: `crypto`
- build phase: passed
- smoke phase: passed
- fuzz duration requested: 5 seconds
- max input length: 4096 bytes
- crash artifacts: 0
- timed out phases: 0
- repository-local orphaned cargo/rustc/wellfriendpdf/cargo-fuzz processes: 0

Artifacts:

- `target/prompt23-writer-crypto/prompt23b-fuzz-target-inventory.json`
- `target/prompt23-writer-crypto/prompt23b-fuzz-smoke-results.json`
- `target/prompt23-writer-crypto/prompt23b-fuzz-process-cleanup.json`
- `target/prompt23-writer-crypto/prompt23b-fuzz-release-verdict.json`

## Security Posture

No crash artifact was emitted under `target/ci-fuzz-artifacts/crypto`. The
crypto fuzz corpus grew during local cargo-fuzz execution; that ignored corpus
state is not committed as release evidence and contains no private keys,
passwords, recovered file keys, MAC keys, or decrypted PubSec seed payloads.
