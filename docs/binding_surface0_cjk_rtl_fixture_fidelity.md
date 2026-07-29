# Multilingual Color Glyphs CJK/RTL Fixture Fidelity

CJK RTL Color Glyph Closeout closes the Korean and Hebrew rendered-page fixture gaps from Roadmap task
10 without changing the PDF text imaging model.

## Korean

The Korean fixture is generated with an embedded Windows Malgun Gothic font. It
covers Hangul syllables, a compatibility jamo code point, a Type0/CIDFontType2
Identity-H path, CIDToGIDMap Identity, and ToUnicode-independent painting.

Evidence:

- `korean-render-fixture-matrix-cjk_rtl_color_glyph_closeout.json`
- `korean-reference-results-cjk_rtl_color_glyph_closeout.json`
- `cjk_rtl_color_glyph_closeout-multi-reference-render-results.json`

## Hebrew

The Hebrew fixture is generated with an embedded Noto Sans Hebrew font. It
covers an explicitly positioned RTL Hebrew run and a mixed LTR/RTL boundary.
Existing PDF glyph streams are painted in PDF-specified glyph order and are not
blindly reshaped. Shaping remains limited to generated/fallback text paths where
Wellfriend owns Unicode-to-glyph layout.

Evidence:

- `hebrew-render-fixture-matrix-cjk_rtl_color_glyph_closeout.json`
- `hebrew-reference-results-cjk_rtl_color_glyph_closeout.json`
- `cjk_rtl_color_glyph_closeout-multi-reference-render-results.json`

## Reference Outcome

The CJK RTL Color Glyph Closeout multi-reference audit records zero Wellfriend outlier failures and
zero unclassified failures for the Korean and Hebrew fixture pages.
