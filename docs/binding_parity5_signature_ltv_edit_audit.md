# Pades LTV Signature LTV/Edit Audit

Schema: `pades_ltv.tsa-dss-ltv-mdp-signature-edits.v1`

## Starting State

- HEAD: `6bc409a5e926d8e6168b3acd07ccf21dd78fb717`
- Branch: `main`
- Worktree before Pades LTV edits: clean at checkpoint, now intentionally dirty with Pades LTV work.
- Required memory cap: 4096 MiB process tree for heavy validation commands.

## Architecture

Pades LTV extends the Signature Validation Resume canonical signature pipeline. Timestamp, DSS/VRI,
PAdES level, and edit-preservation reports are attached to the same per-signature
report rather than a second engine.

## Current Release Posture

Internal implementation, full workspace, binding/package, CLI, WASM, and
historical Codec Boundary-24B gates have passed under the 4096 MiB cap. Release
closure is still blocked by the fact that cargo-fuzz sanitizer smoke was not
completed under the 4 GiB cap.
