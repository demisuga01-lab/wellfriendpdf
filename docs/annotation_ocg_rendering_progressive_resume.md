# Annotation Ocg Rendering Progressive Resume

Annotation Ocg Rendering implements an in-process tile checkpoint model. Renderer Validation closes the validation gap by proving output equivalence, token validation, deterministic resume posture, and cancellation memory accounting.

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

Renderer Validation regression tests:

- `progressive_render_resume_matches_full_page`
- `progressive_resume_token_rejects_mismatched_state`
- `progressive_cancel_report_retains_only_completed_tile_memory`

Artifacts:

- `progressive-resume-equivalence-renderer_validation.json`
- `progressive-resume-invalid-token-renderer_validation.json`
- `progressive-resume-memory-renderer_validation.json`

## Equivalence

Full render and resumed tile render are compared at exact pixel equality in Rust tests. Invalid tokens fail before output reuse. Cancelled jobs retain only completed tiles and report retained bytes.

## Remaining Limits

Binding-level callback APIs and cross-process serialized pixel resumes remain later binding work. Renderer Validation does not claim those surfaces.
