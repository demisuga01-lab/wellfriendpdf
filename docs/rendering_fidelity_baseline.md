# Rendering Fidelity Baseline

Generated for Phase 7 on 2026-07-02.

This is a renderer-fidelity campaign baseline, not a Poppler/PDFium-equivalence
claim. Oxide is still Tier 0 on this stress-weighted slice; the work here
establishes repeatable measurement and closes two small, general mechanisms.

## Method

Reference renderer: Poppler `pdftoppm` 26.02.0.

Oxide binary: `target\release\oxide.exe`, rebuilt before both runs.

Run profile:

- `dpi`: 144
- `max-pages-per-file`: 3
- `max-memory-mb`: 1024
- `threshold-profile`: `renderer`
- deterministic sample: 12 files

Commands:

```powershell
cargo build --release -p oxide-cli
python renderer-benchmark\scripts\renderer_benchmark.py `
  --manifest target\phase7-rendering-corpus-manifest.json `
  --oxide-bin target\release\oxide.exe `
  --dpi 144 --timeout-sec 20 --max-memory-mb 1024 `
  --max-pages-per-file 3 --determinism-sample 12 `
  --output-dir renderer-benchmark\results\phase7-baseline-200

python renderer-benchmark\scripts\renderer_benchmark.py `
  --manifest target\phase7-rendering-corpus-manifest.json `
  --oxide-bin target\release\oxide.exe `
  --dpi 144 --timeout-sec 20 --max-memory-mb 1024 `
  --max-pages-per-file 3 --determinism-sample 12 `
  --output-dir renderer-benchmark\results\phase7-after-final-200
```

The <=200-file corpus is deterministic and deliberately stress-weighted:

| category | files |
| --- | ---: |
| synthetic-graphics | 42 |
| synthetic-transparency | 12 |
| real-complex-vector | 30 |
| real-font-edge | 6 |
| real-cjk-text | 13 |
| real-rtl-text | 5 |
| real-jpeg2000 | 2 |
| real-scanned | 16 |
| real-multi-column | 30 |
| real-forms | 20 |
| synthetic-images | 16 |
| large-files | 8 |

Visual galleries are generated from the benchmark artifacts:

```powershell
python renderer-benchmark\scripts\rendering_fidelity_gallery.py `
  --results-dir renderer-benchmark\results\phase7-baseline-200 `
  --output-dir target\phase7-rendering-gallery\baseline --limit 10

python renderer-benchmark\scripts\rendering_fidelity_gallery.py `
  --results-dir renderer-benchmark\results\phase7-after-final-200 `
  --output-dir target\phase7-rendering-gallery\after-final --limit 10
```

Gallery artifacts are intentionally under `target\`:

- `target\phase7-rendering-gallery\baseline\index.md`
- `target\phase7-rendering-gallery\after-final\index.md`

## Before And After

| metric | baseline | after |
| --- | ---: | ---: |
| files | 200 | 200 |
| visual pages compared | 275 | 274 |
| visual pages passed | 167 | 172 |
| visual pass percent | 60.73% | 62.77% |
| file pass percent | 64.5% | 65.5% |
| weighted score | 52.93 | 54.11 |
| peak Oxide memory | 99.7 MB | 128.59 MB |
| determinism | 12/12 stable | 12/12 stable |

No comparable category pass rate regressed on this slice. One malformed JPX
fixture (`real_pdfjs_bug_jpx`) remained unscored in the final after-run because
Poppler exited while rendering the reference page; the same file rendered in
Oxide and was not part of the code-change improvement claim.

| category | baseline visual pass | after visual pass |
| --- | ---: | ---: |
| large-files | 100.0% | 100.0% |
| real-cjk-text | 56.25% | 56.25% |
| real-complex-vector | 70.97% | 74.19% |
| real-font-edge | 37.5% | 37.5% |
| real-forms | 29.63% | 29.63% |
| real-jpeg2000 | 100.0% | 100.0% |
| real-multi-column | 25.0% | 30.26% |
| real-rtl-text | 40.0% | 40.0% |
| real-scanned | 50.0% | 56.25% |
| synthetic-graphics | 100.0% | 100.0% |
| synthetic-images | 100.0% | 100.0% |
| synthetic-transparency | 100.0% | 100.0% |

Primary failed-page buckets:

| reason | baseline pages | after pages |
| --- | ---: | ---: |
| pixel_difference | 30 | 28 |
| edge_or_text_shift | 25 | 23 |
| large_region_difference | 22 | 21 |
| blank_page_mismatch | 12 | 11 |
| low_ssim | 10 | 10 |
| perceptual_hash_distance | 9 | 9 |
| rendered_page_missing | 3 | 4 |

## Fixed In This Pass

Text/grid fitting:

- Enabled existing light baseline grid fitting for normal body-text glyph sizes
  only for TrueType-backed outlines.
- Kept Type1/CFF outlines, large display text, and tiny captions on the
  analytic outline path. This preserves the Tracemonkey/TeX golden while still
  improving the IRS CID TrueType pages.
- Improved IRS publication pages in `real-multi-column` without moving the
  font-edge category pass rate.

Alpha soft-mask backdrop handling:

- Corrected `/S /Alpha` soft masks with `/BC` so the unpainted mask backdrop can
  become visible when Poppler treats it as opaque.
- Preserved transfer-function cases where `/TR(0)` already provides the visible
  backdrop, avoiding double application.
- Added PDF-level tests for both sides of that rule.

Files that improved in the full run:

- `real_pdfjs_smask_alpha_oob`: 0/1 page passed -> 1/1
- `pdfjs_full_smask_alpha_bc`: 0/1 page passed -> 1/1
- `irs_p15`: 0/3 pages passed -> 1/3
- `irs_p15a`: 0/3 pages passed -> 1/3
- `irs_p15b`: 0/3 pages passed -> 1/3
- `irs_p463`: 2/3 pages passed -> 3/3

## Validation

Commands run after the renderer changes:

- `cargo test --workspace --quiet`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build -p oxide-capi`
- `cargo build -p oxide-wasm --target wasm32-unknown-unknown`
- `python -m maturin build --manifest-path crates/oxide-py/Cargo.toml`
- `dotnet test bindings\dotnet\Oxide.Sdk.Tests\Oxide.Sdk.Tests.csproj --nologo`
- Java FFM smoke with `javac --release 25` and `java --enable-native-access=ALL-UNNAMED`

Extraction slices were unchanged:

- field-F1: `0.72503`
- table shape-F1: `0.96232`
- text char-sim: `0.92743`
- text word-F1: `1.0`

Large-file rendering stayed bounded: the `large-files` category was 24/24
visual pages passed with peak Oxide memory 128.59 MB in the full final run.
The hostile-render spot check over 60 malformed/active-content fixtures was
100% crash-free, timeout-safe, and memory-bounded, with peak Oxide memory
9.07 MB.

## Remaining Gap

The largest remaining bucket is still text/font fidelity:

- `real-multi-column` owns most remaining `pixel_difference` and
  `edge_or_text_shift` failures.
- The current renderer uses analytic glyph outlines and bundled fallback fonts;
  remaining errors are mostly font substitution, hinting, weight, and image/text
  antialiasing differences rather than gross page-geometry failures.

Prompt 04 adds the font subsystem seam for this bucket: deterministic bundled
font-provider reporting, structured font diagnostics, shared renderer text
decoding, generated-output shaping API, and byte-budgeted glyph cache metrics.
It does not claim that font-edge, CJK, or RTL visual parity is fully closed;
those remain measured benchmark targets for font and later color/layout passes.

The next-highest structural bucket is large-region rendering:

- image masks and color-key masks,
- luminosity soft-mask transfer,
- knockout/non-isolated transparency groups,
- some form-heavy IRS pages.

Complex vector/document-program failures remain:

- Type3 cyclic content and tiling-pattern pages with `rendered_page_missing`,
- function-based shading and pattern/font pages with blank or large-region
  mismatches.

Recommended next Phase 7 iteration: attack the text/font bucket first, but with
file-level probes like this pass. If a broader hinting or fallback-font change
regresses `real-font-edge` or Tracemonkey/TAMReview, prefer a narrower font
metric or glyph-positioning mechanism over a global rasterizer change.

## Prompt 03 Architecture Follow-Up

Prompt 03 added a conservative vector display-list and CPU render-device seam in
`crates/engine/src/render/display_list.rs`. The default immediate renderer was
left unchanged for compatibility, while `ContentEngine::build_page_display_list`
and `ContentEngine::render_page_display_list_with_mode` expose the new replay
path for vector-compatible pages.

The Prompt 03 validation run used a small Poppler-backed smoke slice, not the
full Phase 7 stress corpus:

| metric | baseline | after |
| --- | ---: | ---: |
| files | 5 | 5 |
| visual pages compared | 5 | 5 |
| weighted score | 75.0 | 75.0 |
| visual pass | 100.0% | 100.0% |
| determinism | 2/2 stable | 2/2 stable |

The architectural win is replayability and pixel-equivalence for supported
vector pages, verified by unit tests. It is not a new claim that Oxide is
Poppler/PDFium/MuPDF-class.

## Prompt 03B Expanded Renderer Slice

Prompt 03B expanded the reference-renderer check from the five-file synthetic
Prompt 03 smoke to a deterministic 50-file slice covering synthetic vector,
geometry, text, image, transparency, forms, real text, complex vector, forms,
scanned, CJK, RTL, multi-column, JPEG 2000, large-page, and hostile malformed
categories. The run used Poppler `26.02.0`; PDFium was not configured and was
reported as skipped by the benchmark harness.

Command:

```powershell
python renderer-benchmark\scripts\renderer_benchmark.py --manifest target\prompt03b-renderer-slice-manifest.json --oxide-bin target\debug\oxide.exe --dpi 72 --timeout-sec 30 --max-memory-mb 1024 --max-pages-per-file 1 --limit 50 --determinism-sample 5 --threshold-profile renderer --output-dir target\prompt03b-renderer-benchmark
```

Results:

| metric | Prompt 03 smoke | Prompt 03B expanded slice |
| --- | ---: | ---: |
| files | 5 | 50 |
| visual pages compared | 5 | 42 |
| weighted score | 75.0 | 90.45 |
| visual pass | 100.0% | 83.33% |
| hostile files | 0 | 6 |
| hostile crash-free/timeout-safe/memory-bounded | N/A | 100.0% |
| determinism sample | 2/2 stable | 5/5 stable |

Prompt 03B's score is not directly comparable to the five-file Prompt 03 smoke
because the corpus is much broader and intentionally includes real and hostile
fixtures. The important outcomes are broader coverage, explicit display-list
fallback accounting, deterministic re-rendering, and no new renderer crash or
memory-bound failure on the capped slice.

Worst remaining blockers in this slice were `function_based_shading.pdf`,
`IdentityToUnicodeMap_charCodeOf.pdf`, `ThuluthFeatures.pdf`, `bug_jpx.pdf`,
and `jp2k-resetprob.pdf`. Those map to function/shading fidelity, font/text
semantics, and JPEG 2000/image behavior rather than to the Prompt 03B replay
seam itself.

## Prompt 04B Font Slice Rerun

Prompt 04B reran the same 24-file font/text Poppler slice used by Prompt 04:
`real-cjk-text`, `real-font-edge`, and `real-rtl-text`, one rendered page per
file at 72 DPI. The release CLI was rebuilt first.

Command:

```powershell
cargo build --release -p oxide-cli
python renderer-benchmark\scripts\renderer_benchmark.py --manifest renderer-benchmark\corpus\manifest.json --oxide-bin target\release\oxide.exe --category real-cjk-text,real-font-edge,real-rtl-text --output-dir target\prompt04b-font-render-benchmark --dpi 72 --max-pages-per-file 1 --timeout-sec 120 --max-memory-mb 2048 --determinism-sample 5 --threshold-profile renderer
```

Results:

| metric | Prompt 04 | Prompt 04B |
| --- | ---: | ---: |
| files | 24 | 24 |
| visual pages compared | 24 | 24 |
| visual pages passed | 11 | 11 |
| visual pass | 45.83% | 45.83% |
| weighted score | 45.21 | 45.21 |
| peak Oxide memory | 11.72 MB | 11.28 MB |
| determinism | 5/5 stable | 5/5 stable |

Prompt 04B improved the font subsystem's authoring, reporting, CMap, vertical
advance, and explicit fallback behavior, but it did not improve this visual
slice. The remaining failing files are still dominated by glyph rasterizer,
font-substitution, hinting, and text-shape differences against Poppler. The
`real_pdfjs_vertical` artifact is specifically not a score target: Oxide renders
visible vertical text, while the Poppler reference image in this harness is
effectively blank for that fixture.

## Prompt 04C Font Slice Rerun

Prompt 04C reran the same 24-file font/text Poppler slice after implementing
true sfnt/glyf subsetting for generated Type0/CIDFontType2 output and a narrow
non-CID `.notdef`/control-glyph paint guard.

Command:

```powershell
cargo build --release -p oxide-cli
python renderer-benchmark\scripts\renderer_benchmark.py --manifest renderer-benchmark\corpus\manifest.json --oxide-bin target\release\oxide.exe --category real-cjk-text,real-font-edge,real-rtl-text --output-dir target\prompt04c-font-render-benchmark-v2 --dpi 72 --max-pages-per-file 1 --timeout-sec 120 --max-memory-mb 2048 --determinism-sample 5 --threshold-profile renderer
```

Results:

| metric | Prompt 04B | Prompt 04C |
| --- | ---: | ---: |
| files | 24 | 24 |
| visual pages compared | 24 | 24 |
| visual pages passed | 11 | 11 |
| visual pass | 45.83% | 45.83% |
| weighted score | 45.21 | 45.21 |
| peak Oxide memory | 11.28 MB | 11.54 MB |

Local metrics moved in the Standard14/font-edge bucket
(`pdfjs_full_standard_fonts` exact match 89.0344 -> 89.4102), but not enough to
move the aggregate pass threshold. The remaining Prompt 04 visual blockers are
not generated-output embedding problems; they require a focused CFF/Type1C,
CJK/RTL glyph positioning, or raster/hinting fidelity pass, or a formal
acceptance-gate change.

## Prompt 04D Font Slice Rerun

Prompt 04D kept the same original 24-file font/text Poppler slice as the
acceptance anchor and targeted the first high-confidence font-phase-resolvable
failure: `pdfjs_full_glyph_accent.pdf`. Extraction already recovered
`accent U+00E3`, but the renderer missed the Type1C/CFF `atilde` outline because
the glyph was encoded as a `seac` composition and the fallback CFF outline path
did not compose the base/accent outlines.

Command:

```powershell
cargo build --release -p oxide-cli
python renderer-benchmark\scripts\renderer_benchmark.py --manifest renderer-benchmark\corpus\manifest.json --oxide-bin target\release\oxide.exe --category real-cjk-text,real-font-edge,real-rtl-text --output-dir target\prompt04d-font-after-cff-seac --dpi 72 --max-pages-per-file 1 --timeout-sec 120 --max-memory-mb 2048 --determinism-sample 5 --threshold-profile renderer
```

Results:

| metric | Prompt 04C / 04D baseline | Prompt 04D |
| --- | ---: | ---: |
| files | 24 | 24 |
| visual pages compared | 24 | 24 |
| visual pages passed | 11 | 12 |
| visual pass | 45.83% | 50.0% |
| weighted score | 45.21 | 47.5 |
| determinism | 5/5 stable | 5/5 stable |

The changed file was `pdfjs_full_glyph_accent`: fail at 95.35% exact match to
pass at 100.0%. No files regressed. Detailed per-file classification is in
`docs/font_prompt04d_failure_analysis.md`.

## Prompt 04E Final Font Audit Rerun

Prompt 04E kept the exact original 24-file font/text Poppler slice as the
acceptance anchor and audited the remaining concrete font/text fundamentals:
CFF width operands and hint masks, CFF subroutine fallback behavior, PDF
text-state/TJ spacing, Tr invisible/clipping modes, partial ToUnicode fallback,
and Standard14 deterministic substitution.

Command:

```powershell
cargo build --release -p oxide-cli
python renderer-benchmark\scripts\renderer_benchmark.py --manifest renderer-benchmark\corpus\manifest.json --oxide-bin target\release\oxide.exe --category real-cjk-text,real-font-edge,real-rtl-text --output-dir target\prompt04e-font-after-audit --dpi 72 --max-pages-per-file 1 --timeout-sec 120 --max-memory-mb 2048 --determinism-sample 5 --threshold-profile renderer
```

Results:

| metric | Prompt 04D anchor | Prompt 04E audit |
| --- | ---: | ---: |
| files | 24 | 24 |
| visual pages compared | 24 | 24 |
| visual pages passed | 12 | 12 |
| visual pass | 50.0% | 50.0% |
| weighted score | 47.5 | 47.5 |
| peak Oxide memory | 11.99 MB | 11.55 MB |
| determinism | 5/5 stable | 5/5 stable |

The aggregate score did not move beyond Prompt 04D, but no files regressed and
the final font-semantics checklist is now covered by focused tests or explicit
bounded decisions. The unchanged blockers remain CJK/RTL raster or fallback
metric drift, `font_ascent_descent.pdf`, and two blank-reference mismatches that
need another reference renderer or benchmark policy decision. Details are in
`docs/font_prompt04e_final_parity_audit.md`.

## Prompt 05 Color/Prepress Slice

Prompt 05 added a dedicated color/prepress slice using a temporary manifest
under `target/prompt05-color-baseline-manifest.json`. The 24 files cover
synthetic CMYK pages, pdf.js DeviceN/color-space/function fixtures, Indexed
samples, CMYK JPEG, shadings, and tiling patterns.

Command:

```powershell
cargo build --release -p oxide-cli
python renderer-benchmark\scripts\renderer_benchmark.py --manifest target\prompt05-color-baseline-manifest.json --oxide-bin target\release\oxide.exe --dpi 96 --timeout-sec 30 --max-memory-mb 2048 --max-pages-per-file 1 --output-dir target\prompt05-color-after --determinism-sample 4 --threshold-profile renderer
```

Results:

| metric | Prompt 05 baseline | Prompt 05 after |
| --- | ---: | ---: |
| files | 24 | 24 |
| visual pages compared | 23 | 23 |
| visual pass | 60.87% | 60.87% |
| file pass | 58.33% | 58.33% |
| weighted score | 59.0 | 59.0 |
| peak Oxide memory | 19.69 MB | 19.68 MB |
| determinism | 4/4 stable | 4/4 stable |

Prompt 05 did not target new raster fidelity for mesh/pattern/image edge cases,
so the visual score is unchanged. The color work closes architecture,
diagnostic, cap, report, overprint-state, and output-intent validation gaps
without regressing the benchmark.
