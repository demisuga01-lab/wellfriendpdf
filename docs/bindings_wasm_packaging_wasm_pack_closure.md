# Wasm Packaging WASM-Pack Closure

## Starting Checkpoint

- Starting HEAD: `d125e05`.
- Starting worktree: clean.
- Closure scope: only the Release Packaging wasm-pack package artifact that had been recorded as unavailable.

Release Packaging originally completed the WASM target check but left the wasm-pack package artifact as `unavailable` because `wasm-pack` was not on PATH. Wasm Packaging closes that caveat with a target-local bootstrap, real package builds, package inspection, and packaged Node smoke.

## Bootstrap

- Script: `scripts/wasm_packaging_wasm_pack_gate.ps1`.
- Tool version: `wasm-pack 0.13.1`, matching the repository's `^0.13.0` package-tool contract.
- Install method: `cargo install wasm-pack --version 0.13.1 --locked --root target/release_packaging-tools/wasm-pack-0.13.1`.
- Source: crates.io package `wasm-pack/0.13.1`.
- Binary path on Windows: `target/release_packaging-tools/wasm-pack-0.13.1/bin/wasm-pack.exe`.
- Evidence: `target/release_packaging-packaging-codec-isolation/wasm-pack/wasm-pack-bootstrap.json`.

No binary archive is downloaded directly, so there is no external archive checksum to verify. Cargo verifies crates through the registry index and lock-compatible crate checksums.

## Package Build

The gate builds both package forms:

```powershell
target\release_packaging-tools\wasm-pack-0.13.1\bin\wasm-pack.exe build crates/wellfriendpdf-wasm --target web --out-dir target\release_packaging-packaging-codec-isolation\wasm-pack\web-pkg
target\release_packaging-tools\wasm-pack-0.13.1\bin\wasm-pack.exe build crates/wellfriendpdf-wasm --target nodejs --out-dir target\release_packaging-packaging-codec-isolation\wasm-pack\node-pkg
```

Each package contains `wellfriendpdf_wasm_bg.wasm`, JS glue, generated `.d.ts`, package metadata, and the WASM README.

## Inspection

Inspection verifies expected files are present and rejects test fixtures, PDFs, private keys, native libraries, debug junk, and local absolute paths. Evidence is written to:

```text
target/release_packaging-packaging-codec-isolation/wasm-pack/wasm-package-inspection.json
```

## Packaged Node Smoke

The Node smoke imports the generated `nodejs` package directory as a package and verifies package import, `featureReportJson`, opening `minimal.pdf` from bytes, `pageCount`, `securityReportJson`, `codecIsolationReportJson`, invalid input errors, and `close`.

Evidence is written to:

```text
target/release_packaging-packaging-codec-isolation/wasm-pack/wasm-pack-node-smoke.json
```

## Narrow Runtime Fix

The packaged smoke found that the Release Packaging codec isolation report path used `std::process::id()` and `Instant/SystemTime`, which panic on `wasm32-unknown-unknown`. Wasm Packaging keeps native behavior unchanged and uses a deterministic atomic request ID plus zero elapsed milliseconds on WASM report calls.

## Result

The Release Packaging release gate now calls the wasm-pack gate as a required step. The previous `wasm-pack package: unavailable` row is closed and replaced by a passed package build, inspection, and Node smoke.
