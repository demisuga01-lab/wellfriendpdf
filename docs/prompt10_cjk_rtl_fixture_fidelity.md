# Prompt 10 CJK/RTL Fixture Fidelity

Prompt 10B closes the Korean and Hebrew rendered-page fixture gaps from Prompt
10 without changing the PDF text imaging model.

## Korean

The Korean fixture is generated with an embedded Windows Malgun Gothic font. It
covers Hangul syllables, a compatibility jamo code point, a Type0/CIDFontType2
Identity-H path, CIDToGIDMap Identity, and ToUnicode-independent painting.

Evidence:

- `korean-render-fixture-matrix-prompt10b.json`
- `korean-reference-results-prompt10b.json`
- `prompt10b-multi-reference-render-results.json`

## Hebrew

The Hebrew fixture is generated with an embedded Noto Sans Hebrew font. It
covers an explicitly positioned RTL Hebrew run and a mixed LTR/RTL boundary.
Existing PDF glyph streams are painted in PDF-specified glyph order and are not
blindly reshaped. Shaping remains limited to generated/fallback text paths where
Oxide owns Unicode-to-glyph layout.

Evidence:

- `hebrew-render-fixture-matrix-prompt10b.json`
- `hebrew-reference-results-prompt10b.json`
- `prompt10b-multi-reference-render-results.json`

## Reference Outcome

The Prompt 10B multi-reference audit records zero Oxide outlier failures and
zero unclassified failures for the Korean and Hebrew fixture pages.
