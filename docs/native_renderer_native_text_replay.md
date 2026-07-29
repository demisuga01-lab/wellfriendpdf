# Native Renderer Native Text Replay

Text replay is represented by `DisplayOp::NativeTextOp` and executed through
`RenderState::dispatch`, the same renderer path used by immediate rendering.

Covered operators and state:

- Text object lifecycle and text-state operators: `BT`, `ET`, `Tf`, `Td`, `TD`,
  `Tm`, `T*`, `Tc`, `Tw`, `Tz`, `TL`, `Tr`, `Ts`.
- Text showing: `Tj`, `TJ`, single quote, and double quote.
- Fill, stroke, invisible, spacing, rise, horizontal scaling, and line matrix
  posture are preserved through existing `GraphicsState` and text renderer
  behavior.
- CID/CMap/CJK/RTL pages are included in the parity corpus, but raster parity
  for those scripts remains a later renderer campaign item.

Extraction remains independent. Native Renderer does not change ToUnicode provenance
or semantic extraction paths; glyph replay is only a rendering concern.

Evidence:

- `scripts/native_renderer_native_replay_regression.py` asserts native text counters
  on `generated_basic_text.pdf`.
- `target/native_renderer-renderer-native-replay/native-replay-counters.json` records
  aggregate text native replay counts.
- `cargo test -p wellfriendpdf-engine display_list_replays_text_page_through_native_ops`
  verifies immediate-vs-display-list pixel equivalence for a text fixture.

Remaining limits: advanced shaping, hinting parity, full clipping text, and
deep CJK/RTL raster fidelity are bounded to later renderer prompts.
