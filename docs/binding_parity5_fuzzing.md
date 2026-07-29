# PadesLTV Fuzzing

Schema: `pades_ltv.tsa-dss-ltv-mdp-signature-edits.v1`

Pades LTV fuzz bins compile and in-engine hostile seed smoke passes.

Pades LTV Fuzz resolution: sanitizer-backed `cargo-fuzz` now builds and smoke-runs
under the 4096 MiB cap on the current Windows/MSVC toolchain using the default
address sanitizer plus a low-memory dev build recipe
(`CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo +nightly
fuzz run -D --codegen-units 256 --no-trace-compares --disable-branch-folding
false <target> -- -runs=64`). The recipe is baked into `fuzz/Cargo.toml`
`[profile.dev]`. All four Pades LTV signature fuzz targets pass with
SanitizerCoverage active (see the table below); the earlier ASan-OOM and
`--sanitizer none` MSVC-link failures were fuzz-only build/config issues, not
production defects. Details: `docs/pades_ltv_fuzz_cargo_fuzz_root_cause.md` and
`docs/pades_ltv_fuzz_fuzz_blocker_resume_audit.md`.

| Target | Build | Smoke (`-runs=64`, ASan, 4 GiB cap) | SanCov counters |
| --- | --- | --- | ---: |
| `timestamp_token` | pass | pass | 302,209 |
| `signature_preserving_edit_plan` | pass | pass | 337,332 |
| `signature_validation` | pass | pass | 335,694 |
| `signature_evidence` | pass | pass | 43,589 |

Fuzz closure verdict: `closed_cargo_fuzz_passed_current_toolchain`. These four
consolidated, narrow targets cover the nine logical Pades LTV fuzz areas
(timestamp token/attr, DSS/VRI, LTV evidence bundle, MDP permissions,
preserving-edit plan, post-edit revalidation input, VRI keying, DocMDP/FieldMDP
policy); see `target/pades_ltv-signature-ltv-edits/cargo-fuzz-target-inventory-pades_ltv_fuzz.json`.
The canonical fuzz gate remains Linux (`.github/workflows/fuzz.yml`,
`ubuntu-latest`), which now includes these targets.
