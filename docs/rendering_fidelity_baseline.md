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
