# CJK Segmentation

Prompt 06 added CJK character tokenization. Prompt 06B added a bounded simple
segmentation option. Prompt 14 adds optional deterministic dictionary-backed
segmentation while preserving the original behavior and raw extracted text.

## Modes

- `char`: default Prompt 06 behavior; one CJK character per token.
- `simple`: groups contiguous Han, Hiragana, Katakana, or Hangul runs; splits on
  punctuation, script changes, Latin/numeric boundaries, whitespace, and the
  configured max run length.
- `dictionary`: uses the Prompt 14 built-in synthetic fixture dictionary with
  deterministic longest-match segmentation, stable tie-breaking, mixed
  Latin/CJK boundaries, punctuation handling, and unknown-character fallback.

## API

Rust callers set `TextSemanticOptions::cjk_segmentation`.

CLI model-json callers can pass:

```powershell
oxide extract-text --structured --format model-json --cjk-segmentation simple input.pdf
oxide extract-text --structured --format model-json --cjk-segmentation dictionary input.pdf
```

## Guarantees

- Character quads remain the source of token quads.
- Search still maps matches to original character quads.
- `max_cjk_run_chars` bounds long no-space runs.
- Dictionary mode adds the `dictionary_segmented` provenance flag to CJK words.

## Limits

No ML or large bundled dictionary is included. The Prompt 14 built-in dictionary
is a small CC0 synthetic fixture dictionary; production dictionaries should be
user supplied or feature-gated external assets with explicit license metadata.
