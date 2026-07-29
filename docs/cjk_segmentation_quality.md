# CJK Segmentation Quality

CJK Dictionary Layout quality evidence is deterministic fixture evidence plus a
user-pack benchmark harness. It is not a claim that Wellfriend bundles a large
general-purpose dictionary.

Covered fixture classes:

- Chinese words and overlapping entries;
- Japanese words with Han/Kana runs;
- Korean Hangul words;
- mixed Latin/CJK text;
- numbers, punctuation, units, and dates;
- unknown fallback;
- byte and character offset preservation;
- raw extracted text preservation.

Quality artifacts are written under
`target/semantic_intelligence-semantic-intelligence/` and include
`cjk-dictionary-quality-cjk_dictionary_layout.json` and
`cjk-segmentation-fixtures-cjk_dictionary_layout.json`.
