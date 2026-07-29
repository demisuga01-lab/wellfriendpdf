# Semantic Intelligence Local Layout Backend Template

`MockLocalLayoutBackend` is the Semantic Intelligence local backend template. It exercises
the registration, availability, proposal schema, merge, timeout, batch, and
memory-limit path without downloading or loading a real model.

Template fields:

- backend ID and type
- model path and model metadata
- batch page limit
- timeout
- memory limit
- input payload type
- output schema conversion
- unavailable dependency diagnostics

The local mock backend can emit deterministic region proposals from metadata-only
input. Future DocLayNet, LayoutParser, ONNX, or Torch backends can implement the
same schema without changing the core semantic model.

Default posture:

- disabled unless explicitly enabled in configuration
- no external model required for mock tests
- no network use
- no secret handling
- malformed output is rejected by schema validation

Exact limits:

- no ONNX/Torch/LayoutParser runtime is included
- no model-file compatibility is claimed beyond the template contract
- local backend proposals remain optional hints
