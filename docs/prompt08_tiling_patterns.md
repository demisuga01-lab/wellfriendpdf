# Prompt 08 Tiling Patterns

Prompt 08 implements the common native path for PatternType `1` tiling patterns.

Implemented behavior:

- Colored PaintType `1` and uncolored PaintType `2`.
- `/BBox`, `/XStep`, `/YStep`, `/Matrix`, `/Resources`, and content stream
  interpretation.
- Caller color for uncolored patterns through the current color model.
- Pattern cells can contain paths, text, images, Forms, and shadings when their
  resources are supported.
- Cell clipping, recursion depth caps, tile count caps, cancellation polling,
  and scheduler-bounded stream decode are enforced.

Tests:

- `cargo test -p oxide-engine --test patterns --jobs 1`
- Prompt 08 text-clip pattern interaction in
  `cargo test -p oxide-engine --test prompt08_text_clip --jobs 1`

Artifacts:

- `target/prompt08-text-shading-patterns/tiling-pattern-matrix.json`
- `target/prompt08-text-shading-patterns/fallback-taxonomy.json`

Remaining precise limits:

- No unbounded global pattern cache is introduced. Rendering is deterministic
  per render with bounded tile execution.
- Advanced Pattern color-space CMM parity remains later color-management work.
