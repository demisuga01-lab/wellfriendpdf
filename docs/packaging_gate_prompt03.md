# Prompt 03 Packaging Gate

`scripts/prompt03_release_gate.ps1` is the Prompt 03 packaging gate. Prompt 03B updates it so WASM packaging is no longer a soft optional line: it calls `scripts/prompt03b_wasm_pack_gate.ps1`, bootstraps target-local wasm-pack when needed, builds the package, inspects contents, and runs packaged Node smoke.

## Required WASM Step

```powershell
powershell -ExecutionPolicy Bypass -File scripts\prompt03b_wasm_pack_gate.ps1
```

Outputs:

- `target/prompt03-packaging-codec-isolation/wasm-pack/wasm-pack-bootstrap.json`
- `target/prompt03-packaging-codec-isolation/wasm-pack/web-pkg/`
- `target/prompt03-packaging-codec-isolation/wasm-pack/node-pkg/`
- `target/prompt03-packaging-codec-isolation/wasm-pack/wasm-package-inspection.json`
- `target/prompt03-packaging-codec-isolation/wasm-pack/wasm-pack-node-smoke.json`

## Full Gate

```powershell
powershell -ExecutionPolicy Bypass -File scripts\prompt03_release_gate.ps1
```

The release manifest result is:

- `passed` when every required and host-available optional package step passes.
- `passed_with_unavailable_optional` when non-WASM optional ecosystems are missing on the host.
- `failed` when any required step fails.

WASM package generation is required after Prompt 03B and must not be recorded as unavailable on a configured Windows host.
