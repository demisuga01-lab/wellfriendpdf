# Prompt 11 Renderer Fuzz and Metamorphic Campaign

Prompt 11 adds a renderer-specific fuzz campaign and metamorphic suite. The goal
is short, runnable evidence plus infrastructure for later release-duration
fuzzing.

## Fuzz Coverage

The Prompt 11 inventory covers content-stream interpretation, display-list
replay, native text/image/Form replay, transparency groups, soft masks, blend
modes, text clipping, shadings, tiling patterns, annotation appearances,
OCG/layers, progressive state, tile/band/cache paths, color glyph paint graphs,
CJK/RTL paths, malformed resource dictionaries, malformed color spaces, and
renderer scheduler admission.

The additional target is `renderer_prompt11` in `fuzz/fuzz_targets/`. It routes
inputs through structured PDF mutation, display-list parsing/replay surfaces,
font/color report paths, and renderer-adjacent fail-closed code paths.

## Structure-Aware Mutator

`scripts/prompt11_renderer_fuzz_cmm_closeout.py` generates mutated PDF fixtures
under `target/prompt11-renderer-cmm-closeout/renderer-mutator-corpus/`. Mutations
include graphics-state imbalance, CTM/text/image matrix perturbations,
resource changes, soft-mask and blend-mode edits, shading/pattern dictionaries,
OCG references, annotation appearance streams, Type3-like charprocs, clipping,
malformed operands, and deep nesting attempts.

## Metamorphic Tests

`crates/engine/tests/prompt11_renderer_metamorphic.rs` verifies byte-exact RGBA
equivalence for:

- full render versus tiled render
- full render versus banded render
- small versus large tiles
- small versus large bands
- cache disabled, cold cache, and warm cache
- progressive resume versus uninterrupted render
- cancellation/denial recovery

The artifact matrix reports a zero visual tolerance for the synthetic and core
fixture cases. Broader multi-reference close-out metrics keep their own named
tolerance policy.

## Release-Duration Posture

Short smoke runs are required for Prompt 11. Long coverage-guided fuzzing is
deferred as `deferred_release_duration`, not missing infrastructure.
