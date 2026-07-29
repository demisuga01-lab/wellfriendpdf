# Release Packaging Packaging Gate

`scripts/release_packaging_release_gate.ps1` is the Release Packaging packaging gate. Wasm Packaging updates it so WASM packaging is no longer a soft optional line: it calls `scripts/wasm_packaging_wasm_pack_gate.ps1`, bootstraps target-local wasm-pack when needed, builds the package, inspects contents, and runs packaged Node smoke.

## Required WASM Step

```powershell
powershell -ExecutionPolicy Bypass -File scripts\wasm_packaging_wasm_pack_gate.ps1
```

Outputs:

- `target/release_packaging-packaging-codec-isolation/wasm-pack/wasm-pack-bootstrap.json`
- `target/release_packaging-packaging-codec-isolation/wasm-pack/web-pkg/`
- `target/release_packaging-packaging-codec-isolation/wasm-pack/node-pkg/`
- `target/release_packaging-packaging-codec-isolation/wasm-pack/wasm-package-inspection.json`
- `target/release_packaging-packaging-codec-isolation/wasm-pack/wasm-pack-node-smoke.json`

## Full Gate

```powershell
powershell -ExecutionPolicy Bypass -File scripts\release_packaging_release_gate.ps1
```

The release manifest result is:

- `passed` when every required and host-available optional package step passes.
- `passed_with_unavailable_optional` when non-WASM optional ecosystems are missing on the host.
- `failed` when any required step fails.

WASM package generation is required after Wasm Packaging and must not be recorded as unavailable on a configured Windows host.
