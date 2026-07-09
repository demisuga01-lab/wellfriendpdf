# CJK Segmentation Quality

Prompt 14B quality evidence is deterministic fixture evidence plus a
user-pack benchmark harness. It is not a claim that Oxide bundles a large
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
`target/prompt14-semantic-intelligence/` and include
`cjk-dictionary-quality-prompt14b.json` and
`cjk-segmentation-fixtures-prompt14b.json`.
