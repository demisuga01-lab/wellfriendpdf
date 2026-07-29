# CJK Segmentation

Native Renderer added CJK character tokenization. Reference Renderer added a bounded simple
segmentation option. Semantic Intelligence adds optional deterministic dictionary-backed
segmentation while preserving the original behavior and raw extracted text.

## Modes

- `char`: default Native Renderer behavior; one CJK character per token.
- `simple`: groups contiguous Han, Hiragana, Katakana, or Hangul runs; splits on
  punctuation, script changes, Latin/numeric boundaries, whitespace, and the
  configured max run length.
- `dictionary`: uses deterministic longest-match segmentation, stable
  tie-breaking, mixed Latin/CJK boundaries, punctuation handling, and
  unknown-character fallback. The default extraction path uses the small
  built-in fixture provider; CJK Dictionary Layout exposes user-supplied production
  dictionary packs through the provider API.

## API

Rust callers set `TextSemanticOptions::cjk_segmentation`.

Rust callers that need production dictionary terms load a manifest+TSV pack with
`CjkDictionaryProvider::from_manifest_paths` and pass that provider to
`segment_cjk_dictionary_text_with_provider`, `cjk_dictionary_token_search`, or
`cjk_dictionary_rag_token_chunks`.

CLI model-json callers can pass:

```powershell
wellfriendpdf extract-text --structured --format model-json --cjk-segmentation simple input.pdf
wellfriendpdf extract-text --structured --format model-json --cjk-segmentation dictionary input.pdf
```

## Guarantees

- Character quads remain the source of token quads.
- Search still maps matches to original character quads.
- `max_cjk_run_chars` bounds long no-space runs.
- Dictionary mode adds the `dictionary_segmented` provenance flag to CJK words.

## Limits

No ML or large bundled dictionary is included. The built-in dictionary is a
small CC0 synthetic fixture dictionary. Production dictionaries are
user-supplied manifest+TSV packs, or future feature-gated external assets with
explicit license metadata.
