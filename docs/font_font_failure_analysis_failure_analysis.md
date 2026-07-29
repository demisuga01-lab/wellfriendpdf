# Font Failure Analysis Font Benchmark Failure Analysis

Font Failure Analysis reran the exact Font Subsystem Poppler text/font slice:

```powershell
python renderer-benchmark\scripts\renderer_benchmark.py --manifest renderer-benchmark\corpus\manifest.json --wellfriendpdf-bin target\release\wellfriendpdf.exe --category real-cjk-text,real-font-edge,real-rtl-text --output-dir target\font_failure_analysis-font-render-benchmark-v2 --dpi 72 --max-pages-per-file 1 --timeout-sec 120 --max-memory-mb 2048 --determinism-sample 5 --threshold-profile renderer
```

Poppler was available through `pdftoppm`/`pdfinfo` 26.02.0. PDFium was not configured and was skipped. The rerun used the rebuilt release CLI after Font Failure Analysis code changes.

## Summary

| Run | Files | Visual pass | Weighted score | Peak Wellfriend memory |
| --- | ---: | ---: | ---: | ---: |
| Font Subsystem | 24 | 45.83% | 45.21 | 11.28 MB |
| Font Failure Analysis v2 | 24 | 45.83% | 45.21 | 11.54 MB |

Font Failure Analysis did not move the aggregate pass threshold. It did produce local metric movement on the Standard14/font-edge bucket:

| File | 04B exact match | 04C exact match | 04B SSIM | 04C SSIM | 04B MAE | 04C MAE |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `pdfjs_full_standard_fonts` | 89.0344 | 89.4102 | 0.763353 | 0.769361 | 9.9429 | 9.5581 |
| `pdfjs_full_font_ascent_descent` | 95.7867 | 95.8674 | 0.338174 | 0.318664 | 6.5234 | 6.3768 |

The local movement came from skipping true `.notdef`/control-code painting for non-CID simple fonts while preserving custom-named Type1C glyph rendering. It was not enough to change the benchmark pass/fail classification.

## Blocking Files

| File | Category | 04C reason | Font Failure Analysis classification |
| --- | --- | --- | --- |
| `tests/corpus/pdfs/pdfjs/IdentityToUnicodeMap_charCodeOf.pdf` | real-cjk-text | large_region_difference | Text appears visually close in stored artifacts; threshold dominated by raster/edge metric sensitivity. |
| `tests/corpus/pdfs/pdfjs/XiaoBiaoSong.pdf` | real-cjk-text | large_region_difference | CJK glyph/raster/metric drift, not a missing Font Failure Analysis CMap. |
| `tests/corpus/pdfs/pdfjs/ThuluthFeatures.pdf` | real-rtl-text | large_region_difference | Arabic/RTL visual fidelity remains renderer/raster/shaping-display work for existing PDFs. Generated shaped output is handled separately. |
| `tests/corpus/pdfs/pdfjs/issue5801.pdf` | real-rtl-text | blank_page_mismatch | Poppler reference renders blank while Wellfriend renders content; not safe to suppress Wellfriend output as a font fix. |
| `renderer-benchmark/corpus/real-world/pdfjs-full/arial_unicode_en_cidfont.pdf` | real-cjk-text | blank_page_mismatch | Poppler reference renders blank while Wellfriend renders text; not safe to suppress Wellfriend output as a font fix. |
| `renderer-benchmark/corpus/real-world/pdfjs-full/font_ascent_descent.pdf` | real-font-edge | large_region_difference | Embedded Type1C custom glyph names render, but text matrix/advance/raster behavior is still far from Poppler. |
| `renderer-benchmark/corpus/real-world/pdfjs-full/glyph_accent.pdf` | real-font-edge | large_region_difference | Accent/Type1C placement remains a CFF/Type1 glyph positioning fidelity gap. |

## Dominant Buckets

The dominant failing bucket is not generated-output font embedding. True sfnt/glyf subsetting has no effect on rendering existing benchmark PDFs. The measured visual blockers are existing-PDF CJK/RTL/Type1C raster and positioning differences plus two blank-reference mismatches where suppressing Wellfriend output would be incorrect.

Font Failure Analysis therefore closes the production generated-output blocker by implementing true glyf subsetting, but the benchmark-improvement acceptance gate remains unmet. Moving that score materially requires a separate renderer/font-raster fidelity pass focused on CFF/Type1C advances, text matrix placement, and optional native or higher-fidelity raster/hinting decisions.
