# Deterministic And Incremental Writing In Prompt 08

Prompt 08 uses the existing writer instead of inventing another serializer.

Deterministic full rewrite:

- stable object/resource ordering comes from the existing writer modes.
- Prompt 08 tests compare resource digests from repeated identical text
  replacements.
- conversion packages are emitted deterministically for the same input/options
  within the native writer's supported subset.

Incremental save:

- `EditMode::Incremental` appends changed objects after the original bytes.
- tests verify that the original input is preserved as a byte prefix.
- this mode is for additive overlays and simple edits.
- redaction-backed text replacement refuses incremental output because old bytes
  would remain recoverable.

Signature boundary:

- edits can invalidate signatures.
- Prompt 08 does not perform ByteRange/CMS/PAdES/LTV validation.
- Prompt 09 owns full signature preservation and validation decisions.

Versioning helpers:

- `content_defined_chunks` creates bounded rolling-hash chunks.
- `resource_digest` creates SHA-256 resource fingerprints.
- `simhash_text` creates deterministic near-duplicate sketches for text blocks.

Bounded limits:

- object-stream packing and Zopfli-class compression are not Prompt 08 core.
- persistent edit-history storage is not implemented yet.
