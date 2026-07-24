# Prompt 04D Font-Slice Failure Analysis

Prompt 04D started from `846c8dd Implement Prompt 04C glyf subsetting closure` with a clean worktree. The acceptance anchor remained the original Prompt 04B/04C 24-file text/font slice:

```text
python renderer-benchmark\scripts\renderer_benchmark.py --manifest renderer-benchmark\corpus\manifest.json --wellfriendpdf-bin target\release\wellfriendpdf.exe --category real-cjk-text,real-font-edge,real-rtl-text --output-dir target\prompt04d-font-baseline --dpi 72 --max-pages-per-file 1 --timeout-sec 120 --max-memory-mb 2048 --determinism-sample 5 --threshold-profile renderer
```

Poppler 26.02.0 was available. PDFium and MuPDF were not available in this environment. The reproduced baseline matched Prompt 04C: weighted score `45.21`, visual pass `45.83%`, determinism `5/5`.

## Per-File Baseline Classification

| File/page id | Category | Baseline result | Exact % | Primary bucket | Font-phase resolvable | Prompt 04D action |
| --- | --- | ---: | ---: | --- | --- | --- |
| `real_pdfjs_90ms_rksj_h_sample` | CJK text | pass | 100.0 | none | no action | unchanged |
| `real_pdfjs_IdentityToUnicodeMap_charCodeOf` | CJK text | fail | 93.53 | CJK positioning/raster drift | likely later font/rendering | not targeted |
| `real_pdfjs_SimFang-variant` | CJK text | pass | 100.0 | none | no action | unchanged |
| `real_pdfjs_XiaoBiaoSong` | CJK text | fail | 90.269 | CJK raster/metrics drift | likely later font/rendering | not targeted |
| `real_pdfjs_cidfont_cmap_overflow` | CJK text | fail | 98.2 | low SSIM / block geometry | unclear, likely non-font threshold artifact | not targeted |
| `real_pdfjs_issue13343` | CJK text | pass | 100.0 | none | no action | unchanged |
| `real_pdfjs_noembed-eucjp` | CJK text | pass | 100.0 | none | no action | unchanged |
| `real_pdfjs_noembed-jis7` | CJK text | pass | 100.0 | none | no action | unchanged |
| `real_pdfjs_noembed-sjis` | CJK text | pass | 100.0 | none | no action | unchanged |
| `real_pdfjs_vertical` | CJK text | fail | 98.6261 | vertical CJK glyph/raster drift | later vertical alternates | not targeted |
| `real_generated_rtl_placeholder` | RTL text | pass | 100.0 | none | no action | unchanged |
| `real_pdfjs_ArabicCIDTrueType` | RTL text | pass | 100.0 | none | no action | unchanged |
| `real_pdfjs_ThuluthFeatures` | RTL text | fail | 79.0186 | complex RTL font/raster drift | later complex font fidelity | not targeted |
| `real_pdfjs_issue5801` | RTL text | fail | 86.9244 | blank-reference mismatch | not safely font-targeted | classified |
| `real_pdfjs_issue5874` | RTL text | fail | 96.3299 | RTL low SSIM/raster drift | later raster/positioning | not targeted |
| `pdfjs_full_arial_unicode_ab_cidfont` | CJK text | fail | 99.8417 | perceptual hash distance | likely threshold/raster drift | not targeted |
| `pdfjs_full_arial_unicode_en_cidfont` | CJK text | fail | 99.1527 | blank-reference mismatch | reference/language-pack issue likely | classified |
| `pdfjs_full_complex_ttf_font` | font edge | fail | 94.4707 | Arabic/CID TrueType positioning/raster drift | later font fidelity | not targeted |
| `pdfjs_full_Embedded_font` | font edge | pass | 100.0 | none | no action | unchanged |
| `pdfjs_full_font_ascent_descent` | font edge | fail | 95.8674 | Type1C metrics/positioning drift | likely font-resolvable later | not targeted |
| `pdfjs_full_glyph_accent` | font edge | fail | 95.35 | Type1C/CFF accented glyph missing | yes | fixed |
| `pdfjs_full_mmtype1` | font edge | pass | 100.0 | none | no action | unchanged |
| `pdfjs_full_standard_fonts` | font edge | fail | 89.4102 | Standard14 edge/text shift | likely font/raster later | not targeted |
| `pdfjs_full_TrueType_without_cmap` | CJK text | pass | 100.0 | none | no action | unchanged |

## Blank-Reference Classification

`pdfjs_full_arial_unicode_en_cidfont` emits a Poppler warning, `Missing language pack for 'Adobe-Japan1' mapping`, while Wellfriend renders visible content. With no MuPDF/PDFium available locally, this remains classified as a reference/environment limitation or non-font visibility artifact, not a safe Prompt 04D font-code target.

`real_pdfjs_issue5801` emits Poppler font warnings for missing display fonts (`Symbol`, `ArialUnicode`) and malformed ToUnicode warnings. It remains a blank-reference mismatch requiring a separate reference/tooling decision before using it as a font-fidelity gate.

## Targeted Fix

The highest-confidence font-phase-resolvable failure was `pdfjs_full_glyph_accent.pdf`. Extraction already returned `accent U+00E3`, so Unicode recovery was correct. Rendering missed the U+00E3 `atilde` glyph because the embedded Type1C font encoded it as a CFF `seac` composition:

```text
atilde [-51, 148, 0, 97, 196, endchar]
```

The fix adds a bounded CFF fallback for SID-keyed bare CFF simple fonts:

- recover a PDF glyph name to GID through the CFF charset when `ttf-parser` name lookup does not expose it;
- parse only the CFF header, INDEX offsets, Top DICT `Charset` and `CharStrings` offsets, and charset records;
- when `ttf-parser` cannot outline a GID, detect pure `seac` `endchar` composition and resolve the base/accent through CFF StandardEncoding;
- add a small Type2 fallback for simple accent outlines with move/line/curve operators, safe hint-mask skipping, and ignored `12 0`;
- keep subroutines and unsupported operators as clean no-outline fallback instead of guessing.

## Benchmark Movement

After the Type1C/seac fix, the exact original 24-file slice was rerun:

```text
python renderer-benchmark\scripts\renderer_benchmark.py --manifest renderer-benchmark\corpus\manifest.json --wellfriendpdf-bin target\release\wellfriendpdf.exe --category real-cjk-text,real-font-edge,real-rtl-text --output-dir target\prompt04d-font-after-cff-seac --dpi 72 --max-pages-per-file 1 --timeout-sec 120 --max-memory-mb 2048 --determinism-sample 5 --threshold-profile renderer
```

Results:

| Metric | Prompt 04C / 04D baseline | After Prompt 04D fix |
| --- | ---: | ---: |
| Weighted score | 45.21 | 47.5 |
| Visual pass | 45.83% | 50.0% |
| `real-font-edge` pass | 33.33% | 50.0% |
| Determinism | 5/5 | 5/5 |
| Poppler | 26.02.0 | 26.02.0 |

Only `pdfjs_full_glyph_accent` changed: fail `95.35%` exact pixel match to pass `100.0%`. No files regressed.

## Remaining Prompt 04 Limits

- `pdfjs_full_font_ascent_descent` still needs a Type1C metrics/text-state positioning pass.
- CJK/RTL drift files remain dominated by raster, fallback-font, or script-specific positioning differences.
- Vertical punctuation alternates remain a later bounded font-fidelity item.
- Blank-reference mismatches should not be used as Prompt 04 acceptance blockers until the reference-renderer environment is expanded or the pages are reclassified with MuPDF/PDFium evidence.
- Native FreeType/HarfBuzz remains out of the default engine for Prompt 04 because this run proved a pure-Rust CFF fix could move the gate without FFI.
