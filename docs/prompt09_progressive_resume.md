# Prompt 09 Progressive Resume

Prompt 09 implements an in-process tile checkpoint model. Prompt 09B closes the validation gap by proving output equivalence, token validation, deterministic resume posture, and cancellation memory accounting.

## Model

`ProgressiveRenderJob` stores:

- document/page identity
- DPI and render mode
- page pixel dimensions
- tile width/height
- tile cursor
- completed tile buffers
- OCG visibility fingerprint

The resume model is in-process. Completed tile surfaces are retained by the job and are not re-rendered when `render_next()` continues after cancellation or interruption.

## Token Validation

`ProgressiveRenderJob::validate_resume_token()` rejects tokens that differ in page number, DPI, render mode, tile geometry, page dimensions, next tile index, completed tile count, total tile count, non-resumable state, or OCG visibility fingerprint.

Prompt 09B regression tests:

- `progressive_render_resume_matches_full_page`
- `progressive_resume_token_rejects_mismatched_state`
- `progressive_cancel_report_retains_only_completed_tile_memory`

Artifacts:

- `progressive-resume-equivalence-prompt09b.json`
- `progressive-resume-invalid-token-prompt09b.json`
- `progressive-resume-memory-prompt09b.json`

## Equivalence

Full render and resumed tile render are compared at exact pixel equality in Rust tests. Invalid tokens fail before output reuse. Cancelled jobs retain only completed tiles and report retained bytes.

## Remaining Limits

Binding-level callback APIs and cross-process serialized pixel resumes remain later binding work. Prompt 09B does not claim those surfaces.
