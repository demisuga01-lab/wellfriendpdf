# Pades LTV Fuzz — cargo-fuzz Root Cause

Schema: `pades_ltv_fuzz.cargo-fuzz-blocker-closure.v1`

## Classification

The blocker is a **compile-time LLVM/ASan memory limitation** under the 4 GiB
cap, resolvable with fuzz-only build settings — **not** an implementation bug,
**not** a fuzz-target design bug, and **not** an unfixable toolchain blocker. A
secondary, independent issue is a **Windows/MSVC `cargo-fuzz --sanitizer none`
configuration limitation** that is simply avoided by using the supported default
address sanitizer.

| Candidate cause | Verdict |
| --- | --- |
| Unnecessary workspace deps compiled into fuzz targets | Contributing (monolithic engine), but not the lever — see below |
| Fuzz targets pull full engine default features | Features are compile-time no-ops (all `[]`); irrelevant to memory |
| Renderer/Office/bindings pulled into signature fuzzing | N/A — engine is one crate; deps are non-optional regardless |
| Debug info | **Yes** — `-Cdebuginfo=2` is a primary memory driver |
| Link-time memory blowup | No — failure is in rustc/LLVM codegen, not the linker |
| ASan instrumentation memory blowup | **Yes** — with few codegen units |
| SanitizerCoverage on MSVC | Only breaks with `--sanitizer none` (no runtime) |
| Missing compiler-rt/sancov runtime on MSVC | Only relevant to `--sanitizer none` |
| cargo-fuzz/libFuzzer unsupported on host triple | No — works with default ASan + low-memory recipe |
| One oversized fuzz target | No — targets are already narrow |
| Build-script / codegen explosion | No |
| All targets built at once | Mitigated with single-job builds |
| Test helpers pulling pyHanko fixtures into Rust build | No |
| Local toolchain corruption / stale artifacts | No |

## Why engine "features" do not help

`wellfriendpdf-engine`'s Cargo features (`parse`, `render`, `structural`, `sign`,
`pdfa`, `edit`, `create`, `extract`, `ocr`) are all defined as empty (`[]`) —
they are capability/reporting flags, not compile gates. The heavy dependencies
(`reqwest`/`tokio`/`hyper`, image codecs, `rustybuzz`, `cms`/`x509`/`pkix-*`) are
non-optional `[dependencies]` and always compile. Therefore `default-features =
false` or feature trimming cannot shrink the fuzz build. The effective lever is
the LLVM codegen memory profile.

## The two failing configurations (reproduced at HEAD, 4 GiB cap)

1. `--sanitizer address -D --no-trace-compares --codegen-units 16` →
   `rustc-LLVM ERROR: out of memory` compiling `wellfriendpdf-engine` with
   `-Cdebuginfo=2 -Ccodegen-units=16`. Under the 4 GiB process-tree cap this
   exceeds available memory (`hit_memory_cap=true`).
2. `--sanitizer none …` → cargo-fuzz still injects SanitizerCoverage
   (`-Cpasses=sancov-module`, `-Cllvm-args=-sanitizer-coverage-*`) but links no
   sanitizer runtime, so `__start___sancov_pcs` / `__stop___sancov_pcs` section
   boundary symbols (normally supplied by compiler-rt) are undefined →
   `LNK2001 … __stop___sancov_pcs` (many) + `LNK1120: 4 unresolved externals`.
   This is a genuine MSVC limitation of the `--sanitizer none` path; it is not a
   memory issue and must **not** be "fixed" with fake `__sancov_pcs` stubs
   (that would silently disable coverage and is not a valid cargo-fuzz pass).

## The fix (proven, fuzz-only, production untouched)

Keep the **default address sanitizer** — its runtime resolves the sancov
section symbols, so linking succeeds on MSVC — and cut LLVM peak memory:

```
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 \
  cargo +nightly fuzz build/run -D --codegen-units 256 --no-trace-compares \
  --disable-branch-folding false <target> [-- -runs=64 -max_len=262144]
```

- `-D` (dev profile) + `debuginfo=0`: drops the debug-info memory driver.
- `--codegen-units 256`: splits the monolithic crate into many small units so
  per-unit LLVM peak stays far under the cap.
- `CARGO_BUILD_JOBS=1` + incremental off: bounds concurrent rustc memory.
- `--no-trace-compares` / `--disable-branch-folding false`: reduce instrumentation
  weight while keeping SanitizerCoverage counters/PC tables.

This recipe is baked in durably as `[profile.dev]` (`debug = 0`,
`incremental = false`, `codegen-units = 256`) in `fuzz/Cargo.toml`. It affects
only the out-of-workspace fuzz crate's `--dev` builds — not the workspace and not
the release fuzz profile the Linux CI gate uses. No production code is changed.

## Result under the 4 GiB cap (current Windows/MSVC toolchain)

| Target | Build | Smoke (`-runs=64`) | SanitizerCoverage counters | cov (done) |
| --- | --- | --- | ---: | ---: |
| `timestamp_token` | pass | pass (exit 0) | 302,209 | 1,175 |
| `signature_preserving_edit_plan` | pass | pass (exit 0) | 337,332 | 458 |
| `signature_validation` | pass | pass (exit 0) | 335,694 | 536 |
| `signature_evidence` | pass | pass (exit 0) | 43,589 | 793 |

Cold engine build finished in ~6m13s under the cap (`hit_memory_cap=false`);
fuzz child RSS stayed ~45–55 MB during the smokes. No crashes, hangs, OOMs, or
false-valid results. Evidence: `cargo-fuzz-build-results-pades_ltv_fuzz.json`,
`cargo-fuzz-smoke-results-pades_ltv_fuzz.json`, `cargo-fuzz-memory-results-pades_ltv_fuzz.json`,
`fuzz-closure-verdict-pades_ltv_fuzz.json`.
