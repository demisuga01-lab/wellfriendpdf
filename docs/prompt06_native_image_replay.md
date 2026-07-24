# Prompt 06 Native Image Replay

Image replay is represented by:

- `DisplayOp::NativeImageXObject` for `/XObject` resources whose subtype is
  `/Image`.
- `DisplayOp::NativeInlineImage` for `BI`/`ID`/`EI` inline image payloads.
- `DisplayOp::StateOp` for graphics-state changes such as `cm`, colors,
  dash, and ExtGState that must be replayed before native image operations.

Image XObject subtype discovery is added in `PageResources`, so image and Form
XObject invocation no longer relies on resource-name guessing. Actual image
decode stays in the existing renderer path (`handle_do`, inline image paint,
scheduled decode helpers), preserving Prompt 04/05 scheduler admission and image
limits rather than adding a second decode route.

Evidence:

- `scripts/prompt06_native_replay_regression.py` asserts Image XObject and
  inline image native counters with zero compatibility fallback.
- The audit corpus includes `generated_image_only.pdf` and a generated inline
  image PDF.
- `cargo test -p wellfriendpdf-engine display_list_replays_image_page_through_native_ops`
  verifies immediate-vs-display-list pixel equivalence for an image fixture.

Remaining limits: image masks, soft masks, interpolation parity, ICC nuance,
and complex color conversions are reported/postured but not completed here.
Those remain bounded to later image, transparency, and color renderer prompts.
