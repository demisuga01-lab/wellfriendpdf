# Prompt 06 Form XObject Replay

Form replay is represented by `DisplayOp::NativeFormXObject` for `/XObject`
resources whose subtype is `/Form`. The operation is executed through
`RenderState::dispatch` and `handle_do`, preserving the existing form resource
stack, matrix, bbox, and nested content behavior rather than duplicating form
interpretation in the display-list builder.

State preservation needed for Form replay is handled by:

- `DisplayOp::Save` and `DisplayOp::Restore`, which now preserve the
  `GraphicsState` stack during `RenderState` display-list replay.
- `DisplayOp::StateOp`, which replays graphics-state mutation before native
  high-level operations.
- `PageResources::xobject_subtypes`, which distinguishes Image and Form
  resources deterministically.

Evidence:

- `scripts/prompt06_native_replay_regression.py` asserts a native Form XObject
  counter on `renderer-benchmark/corpus/synthetic/synthetic_form_000.pdf`.
- The parity corpus includes simple and nested synthetic Form XObject pages.
- Form native replay counts are recorded in
  `target/prompt06-renderer-native-replay/native-replay-counters.json`.

Remaining limits: full transparency-group compositing, exotic annotation
appearance parity, pattern-cell forms, and form cache promotion remain later
renderer work. Prompt 06 preserves metadata and fallback visibility rather than
claiming those as complete.
