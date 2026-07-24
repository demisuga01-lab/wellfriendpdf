# Stability and SemVer Policy

Wellfriend currently publishes workspace crates at version `0.1.0`. That is an
honest pre-1.0 signal: the stable integration path is documented, but some
low-level PDF internals can still move before a `1.0` commitment.

## What Is Stable

The following are the intended stable surfaces during the `0.x` line:

- `wellfriendpdf_engine::prelude`
- `ContentEngine`
- canonical parse types: `Document`, `Page`, `Block`, `ParseOptions`,
  `SerializeOptions`, extraction profiles, and region/scoped extraction types
- authoring/editing/compliance/signature option structs documented in
  `docs/api_overview.md`
- `WellfriendError`, `ErrorKind`, `Result<T>`, and `WellfriendError::code()`
- CLI command names, exit codes, and documented JSON output schemas in
  `README.md`, `docs/cli.md`, and `wellfriendpdf --help`
- Python binding package/module surface documented in `docs/python_binding.md`
  and `crates/wellfriendpdf-py/README.md`
- C ABI symbols in `crates/wellfriendpdf-capi/include/wellfriendpdf.h`
- WASM exports documented in `docs/bindings.md`
- HTTP `/api/v1/*` endpoint paths and documented JSON response shapes

## What Is Experimental

The crate root exposes a broad set of PDF internals for power users. Modules
such as `content`, `filters`, `fonts`, `images`, `object`, `parser`, `reader`,
`render`, and `writer` are public but lower-level. They may change before 1.0
when needed to keep the high-level SDK coherent.

## SemVer Rules

- Patch releases fix bugs and may add non-breaking APIs.
- Minor `0.x` releases may adjust experimental internals.
- Stable-surface removals or behavior changes require a changelog entry, a
  migration note, and a clear reason.
- Deprecated stable APIs should remain for at least one minor release before
  removal, unless they are unsound or security-sensitive.
- `rust-version` bumps are allowed in minor `0.x` releases, must be documented,
  and should be driven by a dependency or language-feature need rather than
  convenience.
- After `1.0`, SemVer-compatible releases must not remove or rename stable
  public APIs, CLI commands, C ABI symbols, Python methods, WASM exports, or
  HTTP endpoints without a major version bump.

## MSRV

Minimum supported Rust version: **1.95**.

Every workspace crate pins `rust-version = "1.95"`. This was verified with:

```text
rustc 1.95.0 (59807616e 2026-04-14)
cargo 1.95.0 (f2d3ce0bd 2026-03-21)
```

The MSRV policy for the pre-1.0 line:

- CI and release checks should build/test on Rust 1.95 or newer stable.
- MSRV bumps are documented in this file and the release notes.
- MSRV bumps are not treated as breaking changes before 1.0, but they should
  not happen casually.
- A future 1.0 release should state whether MSRV bumps are minor-version
  changes or require a longer support window.

## API Drift Checks

Recommended future CI guard:

```sh
cargo install cargo-public-api
cargo public-api -p wellfriendpdf-engine --simplified > docs/public-api-wellfriendpdf-engine.txt
```

Run the command before releases and review the diff. `cargo-semver-checks` can
be added once the crate commits to a `1.0` stable surface.

## Path To Enterprise-Grade

This repository can pin policy, tests, and security posture in code, but three
enterprise prerequisites remain outside what a prompt can honestly complete:

1. An external, independent third-party security audit of the parser/rendering,
   crypto/signature, server, C ABI, and supply-chain surfaces.
2. A sustained real-world robustness track record on large wild-PDF corpora and
   pilot customer documents. The current robustness runs are useful evidence,
   but they are not a long-term production track record.
3. Any formal compliance certification a customer requires. Certification is a
   process and evidence exercise, not a code-only claim.

Until those are done, the honest status is: code-level enterprise hardening is
documented and test-backed; enterprise certification/audit status is not claimed.
