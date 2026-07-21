# Prompt 25B — cargo-fuzz Blocker Resume Audit

Schema: `prompt25b.cargo-fuzz-blocker-closure.v1`

Prompt 25B resumes the incomplete Combined Prompt 25 run to resolve the single
outstanding closure gap — sanitizer-backed `cargo-fuzz` execution under the
4 GiB memory cap — without redoing Prompt 25 and without discarding any work.

## Starting state (preserved, not modified)

- HEAD: `6bc409a5e926d8e6168b3acd07ccf21dd78fb717`
  ("Close combined prompt 24 certificate trust pades ocsp crl").
- Branch `main`, remote `https://github.com/demisuga01-lab/oxide-parser.git`.
- Worktree was dirty with the full Prompt 25 implementation (signature/timestamp/
  DSS/VRI/LTV/MDP/edit surfaces across Rust, CLI, Python, C ABI, .NET, Java, WASM),
  plus untracked Prompt 25 docs, scripts, two new fuzz targets, and gitignored
  `target/prompt25-signature-ltv-edits/` evidence.
- `git diff --check` / `git diff --cached --check`: clean.
- Host: Windows 11 (10.0.26200), x86_64, 16 GB RAM.
- Toolchains: rustc nightly `1.98.0-nightly (423e3d252 2026-05-24)` (LLVM 22.1.6),
  `cargo-fuzz 0.13.1`; stable `1.95.0` default. Installed targets:
  `wasm32-unknown-unknown`, `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`.
- Processes at start: only persistent `dotnet.exe` MSBuild build-server nodes; no
  repo-local long-running cargo/rustc/oxide/fuzz/java/python processes.

Full machine-readable start state:
`target/prompt25-signature-ltv-edits/prompt25b-resume-start-state.json`.

## Recovery snapshot (before any change)

An external recovery archive was created outside the repo and hash-recorded:

- Path: `E:\wellpdfsdk-prompt25-recovery\prompt25b-fuzz-blocker-resume-20260721T115902.zip`
- SHA-256: `e2cecf630418b80b7861bf20e06bc7c07880dea95f4a25f616234a65437a26e2`
- Size: 43,012,774 bytes; sidecar `<zip>.sha256.json`.
- Contents: `tracked-dirty.patch`, `tracked-dirty-files/`, `untracked-files/`,
  `generated-prompt25-artifacts/` (80.85 MB), `inventory.json`.

The archive is not committed and is not a substitute for the closure commit.
See `target/prompt25-signature-ltv-edits/prompt25b-recovery-artifact.json`.

## Blocker reproduction (fresh, at HEAD, under the 4 GiB Job Object cap)

The 4 GiB cap is enforced with a Windows Job Object
(`JOB_OBJECT_LIMIT_JOB_MEMORY`) through `scripts/large_file_profile.py exec
--memory-limit-mb 4096`, which caps the whole process tree and samples memory.

1. **ASan OOM** — `cargo +nightly fuzz run timestamp_token --sanitizer address
   -D --no-trace-compares --codegen-units 16 -- -runs=1 -max_len=256 -timeout=5`
   → `rustc-LLVM ERROR: out of memory` / `Allocation failed` while compiling
   `oxide-engine`; `exit_code=1`, `hit_memory_cap=true`, ~100 s.
   Log: `cargo-fuzz-asan-oom-repro.log`.
2. **`--sanitizer none` MSVC link failure** — same target with `--sanitizer
   none` → many `LNK2001: unresolved external symbol __stop___sancov_pcs` and
   `LNK1120: 4 unresolved externals`; `exit_code=1`, ~13 s.
   Log: `cargo-fuzz-none-msvc-link-repro.log`.

These match the two failing commands recorded during the incomplete Prompt 25
run at this HEAD (`fuzz/large-file-profile/results/20260721-045337-*` and
`…-045455-*`). See `cargo-fuzz-blocker-reproduction.json`.

## Resolution

Root cause and the proven fix are documented in
`docs/prompt25b_cargo_fuzz_root_cause.md`. In short: keep the **default address
sanitizer** (its runtime provides the `__sancov_pcs` section-boundary symbols
that `--sanitizer none` leaves undefined on MSVC) and cut LLVM peak memory with
fuzz-only build knobs (`debuginfo=0`, `codegen-units=256`, incremental off,
single build job, `--no-trace-compares`). This was applied durably as a
`[profile.dev]` block in `fuzz/Cargo.toml` and re-proven under the 4 GiB cap on
the current Windows/MSVC toolchain: all four Prompt 25 signature fuzz targets
build and smoke-run (`-runs=64`) with SanitizerCoverage active.

Fuzz closure verdict: `closed_cargo_fuzz_passed_current_toolchain`
(`fuzz-closure-verdict-prompt25b.json`).

## Project fuzz policy

The canonical fuzz environment per `.github/workflows/fuzz.yml` and
`.github/workflows/differential-fuzz.yml` is `ubuntu-latest` (Linux) nightly
cargo-fuzz; Windows/MSVC cargo-fuzz was never a release requirement. Prompt 25B
additionally adds the Prompt 24/25 signature targets (`signature_evidence`,
`timestamp_token`, `signature_preserving_edit_plan`) to that Linux gate.
