# Prompt 04 RLBox/WASM Sandbox Feasibility

## Verdict

RLBox/WASM codec sandboxing is hard-blocked for Prompt 04. Wellfriend must not claim RLBox support from documentation alone.

The practical sandbox boundary remains the Prompt 03 `wellfriendpdf-codec-worker` subprocess protocol with input caps, output caps, timeout handling, worker crash containment, and fail-closed policy modes.

## Evidence

The Prompt 04 feasibility wrapper records local command evidence in:

`target/prompt04-codec-boundary-scheduler/rlbox-wasm-feasibility.json`

The local Windows probe found:

- `cmake` and `cargo` available;
- `wasm32-unknown-unknown` installed;
- `emcc`, `clang++`, and `wasm-pack` not available on PATH during the feasibility probe;
- no existing RLBox/WASM sandbox integration in the repository inventory;
- `cargo search rlbox` and `cargo search rlbox-wasm` produced no usable Rust integration candidate output in this run.

## Why A Stub Was Not Built

A minimal RLBox/WASM codec stub would need a reproducible C/C++ to WASM toolchain, a sandbox runtime or binding layer, host packaging, and cross-platform CI coverage. Those prerequisites were not available locally, so building a partial document-only prototype would overclaim support.

## Future Path

Future work should be isolated from `wellfriendpdf-engine`:

- create a separate optional experiment crate;
- compile a no-op C codec stub to WASM;
- prove Windows/Linux/macOS CI builds;
- prove call-boundary copying, error returns, and bounded memory;
- add release packaging evidence before integrating any real codec;
- keep native registry entries worker-required and deny-by-default.
