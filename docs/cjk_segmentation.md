# CJK Segmentation

Prompt 06 added CJK character tokenization. Prompt 06B adds a bounded simple
segmentation option while preserving the original behavior.

## Modes

- `char`: default Prompt 06 behavior; one CJK character per token.
- `simple`: groups contiguous Han, Hiragana, Katakana, or Hangul runs; splits on
  punctuation, script changes, Latin/numeric boundaries, whitespace, and the
  configured max run length.
- `dictionary`: currently aliases `simple`. A later API can add a user-provided
  dictionary without changing the mode shape.

## API

Rust callers set `TextSemanticOptions::cjk_segmentation`.

CLI model-json callers can pass:

```powershell
oxide extract-text --structured --format model-json --cjk-segmentation simple input.pdf
```

## Guarantees

- Character quads remain the source of token quads.
- Search still maps matches to original character quads.
- `max_cjk_run_chars` bounds long no-space runs.

## Limits

No ML or large bundled dictionary is included. Japanese/Chinese/Korean word
boundaries are approximate in `simple` mode and confidence remains lower than
space-delimited Latin words.

