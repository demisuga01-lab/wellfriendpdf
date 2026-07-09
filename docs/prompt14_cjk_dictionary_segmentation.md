# Prompt 14 CJK Dictionary Segmentation

Prompt 14 keeps the existing CJK segmentation baseline and adds optional
dictionary-backed tokenization for the semantic word layer.

Modes:

- `char`: default deterministic character tokenization
- `simple`: bounded script-run tokenization
- `dictionary`: deterministic longest-match tokenization with unknown fallback

Dictionary mode does not rewrite extracted raw text. It only changes the token
layer used by semantic words, search/RAG consumers that opt into the semantic
model, table cell token views, and figure/caption token views.

Built-in dictionary policy:

- name: `oxide-prompt14-synthetic-cjk-test-dictionary`
- license: `CC0-1.0 synthetic fixture terms`
- scope: small test/fixture terms for Chinese, Japanese, Korean, and mixed text
- no large third-party dictionary is bundled
- user dictionaries can be loaded as external metadata; their license remains
  user supplied and report-visible

Algorithm:

- maximum longest-match over the configured dictionary
- stable dictionary order tie-breaking
- Han/Hiragana/Katakana runs can match Japanese mixed-script terms
- Hangul runs are kept script-aware
- Latin, numbers, and units are separated at script boundaries
- punctuation is tokenized in dictionary mode
- unknown CJK text falls back deterministically to single-character tokens

Each dictionary token carries source offsets, confidence posture, and the
`dictionary_segmented` provenance flag. Bounding boxes are aggregated from the
source character quads.

Exact limits:

- the bundled dictionary is intentionally small
- production dictionaries should be user supplied or feature-gated external assets
- dictionary segmentation is not morphological analysis
- raw text, redaction evidence, and deterministic extraction remain unchanged
