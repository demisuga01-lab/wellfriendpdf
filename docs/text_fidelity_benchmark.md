# Text Character Fidelity Benchmark

**Plain-language summary.** Prompt 4 found that Wellfriend's low character similarity was dominated by reading-order and line-structure issues, not missing words: the words were usually present, but sparse page furniture could be moved after body text and three-column rows could be emitted as joined visual rows. The fix tightens default two-column detection and adds a narrow structured-text fallback for obvious row-joined columns. On the indicative (approx 200-file subset) text-heavy loop, char-sim improved from 0.686 to 0.765 while word-F1 stayed effectively flat at 0.910; this is real progress, but Wellfriend remains below the current leaders around 0.92 on this slice.

## Scope

These numbers are indicative (approx 200-file subset), not final benchmark claims. Final validation belongs in Prompt 10.

Two deterministic subsets were used:

- Required fast loop: first 200 files from `test_corpus/`, sorted by filename.
- Diagnosis loop: first 200 files from the `text-heavy` category, sorted by filename.

All runs used the crash-safe benchmark harness with `--max-workers 4`, `--timeout 60`, and `--max-memory-mb 2048`.

## Diagnosis

The scorer normalizes spaces and repeated blank lines before `SequenceMatcher` character similarity, while word-F1 tokenizes normalized text. That explains the original metric shape: high word-F1 with low char-sim means the text vocabulary is present but the character stream is in a different order or line structure.

The recurring failures were:

1. Sparse page furniture misread as a second column. Example: titles or document identifiers on one side of the page were emitted after body text.
2. Three-column visual rows emitted as one line with double-space gaps. Example: `pdf_000019` default extraction joined left/middle/right column rows; structured extraction recovered column-first reading order.
3. Remaining residuals: metadata/header placement and harder mixed layouts where structured output is sometimes better and sometimes worse. A broad structured fallback was rejected because it regressed many already-good files.

## Changes

- `ReadingOrderReconstructor::find_column_split_x` now requires repeated aligned left/right baselines before accepting a two-column split. This keeps true row-aligned columns while ignoring sparse page furniture.
- `ContentEngine::get_page_text` now uses the existing structured layout analyzer only when default output has a strong row-joined-column signature: many nonempty lines with double-space joins and very few genuinely long lines.
- Added regression tests for sparse page furniture and the structured fallback trigger.

## Indicative Results

### Required First-200 Subset

| metric | before | after |
| --- | ---: | ---: |
| pass rate | 100.0% | 100.0% |
| char-sim | 0.912 | 0.927 |
| word-F1 | 1.000 | 1.000 |
| line recall | 1.000 | 1.000 |
| spurious ratio | 0.066 | 0.076 |
| order | 0.957 | 0.960 |

### Text-Heavy 200-File Diagnosis Subset

| metric | before | after |
| --- | ---: | ---: |
| pass rate | 100.0% | 100.0% |
| char-sim | 0.686 | 0.765 |
| word-F1 | 0.910 | 0.910 |
| line recall | 0.996 | 0.996 |
| spurious ratio | 0.267 | 0.246 |
| order | 0.960 | 0.965 |

Leader reference from the baseline text-heavy run: pypdf 0.923 char-sim, PyMuPDF 0.922, pdftext 0.921. Wellfriend is still behind them on this text-heavy slice.

## Regression Checks

| check | result |
| --- | --- |
| has-tables first-200 subset | table cell-F1 0.858, pass 100.0% |
| has-fields first-200 subset | strict field-F1 0.663, value-only F1 0.748 |
| robustness corpus, Wellfriend only | survival 100.0%, parsed-pass 87.5%, hard failures 0 |

## Commands

```powershell
cargo build --release -p wellfriendpdf-cli
python extraction-benchmark\scripts\competitive_benchmark.py --corpus E:\wellpdfsdk\test_corpus --limit 200 --tools wellfriendpdf --tasks text --output-dir target\competitive-benchmark\prompt4-text-final-first200 --report target\competitive-benchmark\prompt4-text-final-first200.md --max-workers 4 --timeout 60 --max-memory-mb 2048
python extraction-benchmark\scripts\competitive_benchmark.py --corpus E:\wellpdfsdk\test_corpus --category text-heavy --limit 200 --tools wellfriendpdf --tasks text --output-dir target\competitive-benchmark\prompt4-text-final-textheavy200 --report target\competitive-benchmark\prompt4-text-final-textheavy200.md --max-workers 4 --timeout 60 --max-memory-mb 2048
python extraction-benchmark\scripts\competitive_benchmark.py --corpus E:\wellpdfsdk\test_corpus --category has-tables --limit 200 --tools wellfriendpdf --tasks tables --output-dir target\competitive-benchmark\prompt4-regression-tables --report target\competitive-benchmark\prompt4-regression-tables.md --max-workers 4 --timeout 60 --max-memory-mb 2048
python extraction-benchmark\scripts\competitive_benchmark.py --corpus E:\wellpdfsdk\test_corpus --category has-fields --limit 200 --tools wellfriendpdf --tasks fields --output-dir target\competitive-benchmark\prompt4-regression-fields --report target\competitive-benchmark\prompt4-regression-fields.md --max-workers 4 --timeout 60 --max-memory-mb 2048
python robustness-benchmark\scripts\robustness_benchmark.py --manifest robustness-benchmark\manifest.json --wellfriendpdf-bin target\release\wellfriendpdf.exe --output-dir target\robustness-benchmark\prompt4-regression --report target\robustness-benchmark\prompt4-regression.md --tools wellfriendpdf --timeout 60 --max-memory-mb 2048 --max-workers 4
```

## Remaining Gap

The residual char-sim gap is mostly line/order fidelity in mixed layouts. The existing structured analyzer is capable of fixing some cases, but it is not safe to apply globally because it can move identifiers or blocks in files where the default extractor is already close to ground truth. Future work should make the structured analyzer's block precedence more reliable, then widen the fallback or make structured extraction the default.
