# Multilingual Color Glyphs Hinting Posture

Color Glyph Hinting keeps the default renderer pure Rust and records a reference-cluster
acceptance proof instead of adding a native hinting dependency.

No native hinting backend is added, no native dependency is enabled silently, and
WASM/default builds remain portable. Feature reports expose the active posture
as `pure_rust_analytic_aa`; optional native hinting remains a future
feature-gated enhancement, not a Multilingual Color Glyphs blocker.

Evidence:

- `hinting-posture-cjk_rtl_color_glyph_closeout.json`
- `hinting-posture-color_glyph_hinting.json`
- `cjk_rtl_color_glyph_closeout-multi-reference-diff-metrics.json`
- `cjk_rtl_color_glyph_closeout-reference-disagreement-summary.json`
- `multi-reference-diff-metrics-color_glyph_hinting.json`
- `reference-disagreement-summary-color_glyph_hinting.json`

The Color Glyph Hinting rendered corpus includes Korean, Hebrew, COLRv1, sbix, and
CID-keyed CFF regression rows. It records zero Wellfriend outlier failures and zero
unclassified failures under the pure-Rust posture.
