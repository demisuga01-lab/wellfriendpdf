# Prompt 06 Renderer Failure Taxonomy

The machine-readable taxonomy is written to
`target/prompt06-renderer-native-replay/failure-taxonomy.json`.

Reference execution categories:

- `missing_binary`: the reference executable was not found.
- `reference_execution_failure`: the reference process exited non-zero.
- `render_timeout`: the reference or Oxide render exceeded its timeout.
- `blank_output`: a reference command succeeded but produced no usable output.
- `malformed_input_rejection`: the input is rejected before comparison.

Comparison categories:

- `reference_disagreement`: available references disagree with each other.
- `oxide_mismatch`: Oxide output differs from the agreed reference result.
- `unsupported_comparison`: no usable reference output exists.
- `visual_pass_with_compatibility_fallback`: visual comparison can proceed, but
  the display-list report still contains measured fallback.
- `native_replay_audited`: Oxide rendered through the native replay path for the
  covered operation category.

Fallback reasons:

- `unsupported_operator_shading`
- `unsupported_operator_pattern`
- `unsupported_graphics_state`
- `unsupported_xobject_subtype`
- `malformed_content`
- `safety_limit_exceeded`

Future renderer prompts should add categories only when they are needed for a
new failure mode. They should not merge reference failures, Oxide mismatches,
and unsupported comparisons into one generic bucket.
