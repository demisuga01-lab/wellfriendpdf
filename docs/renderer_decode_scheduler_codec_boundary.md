# Codec Boundary Renderer Decode Scheduler Adoption

## Scope

Codec Boundary adopts the decode scheduler boundary for renderer decode work without changing rendering semantics or enabling opportunistic parallel composition.

The renderer now owns a per-render `RenderDecodeScheduler` context that uses `DecodeMemoryBudget`. Decode jobs run synchronously in the immediate renderer so output order remains deterministic, but every adopted path acquires a memory token before decoding and checks cancellation before starting decode work.

## Adopted Paths

Covered renderer paths:

- image XObject decode;
- inline image decode;
- stencil image mask decode through the inline/XObject paths;
- soft-mask image decode;
- Form XObject stream decode;
- transparency Form XObject stream decode;
- annotation appearance stream decode;
- tiling pattern stream decode;
- mesh shading stream decode;
- tile and band rendering through the shared full-page render path.

`RenderCache` remains a final tile-buffer cache, not a decoded image-stream cache. Codec Boundary does not introduce speculative parallel renderer predecode.

## Memory And Cancellation

Decode memory estimates are conservative reservations based on declared image dimensions or capped stream raw-size expansion estimates. Existing image dimension and decoded-output caps remain the hard allocation guards.

If a token cannot be acquired, decode fails closed with a scheduler budget error. If the render `CancelToken` is already cancelled, decode returns `WellfriendError::Cancelled` before entering decoder work.

## Determinism

The renderer still composes in content-stream order. The scheduler is used for memory admission and metrics, not for racing paint operations. The Codec Boundary report generator renders the `image_only.pdf` fixture twice and records matching output hashes.

## Evidence

Tests:

- `renderer_inline_decode_acquires_scheduler_token`
- `renderer_decode_scheduler_fails_closed_over_budget`
- `renderer_decode_scheduler_observes_cancel_before_decode`

Artifacts:

- `target/codec_boundary-codec-boundary-scheduler/renderer-scheduler-report.json`

Public posture report:

- `renderer_decode_scheduler_adoption_report()`
