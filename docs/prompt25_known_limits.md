# Prompt25 Known Limits

Schema: `prompt25.tsa-dss-ltv-mdp-signature-edits.v1`

Current exact limits: no B-LTA promotion, no public general signing workflow, supported edit family is append-only form fill plus existing DSS evidence posture, and WASM remains offline/host-supplied only for constrained operations.

cargo-fuzz toolchain limit (Prompt 25B): on Windows/MSVC, sanitizer-backed
cargo-fuzz must use the default address sanitizer plus the low-memory dev recipe
(`-D --codegen-units 256 --no-trace-compares` with `CARGO_PROFILE_DEV_DEBUG=0`
and `CARGO_BUILD_JOBS=1`) to build under a 4 GiB cap; the `--sanitizer none`
path is unsupported on MSVC because cargo-fuzz still emits SanitizerCoverage but
links no runtime (`__sancov_pcs` symbols unresolved). Coverage-guided fuzzing
under the 4 GiB cap otherwise passes on the current toolchain, and the canonical
CI fuzz gate is Linux (`ubuntu-latest`).
