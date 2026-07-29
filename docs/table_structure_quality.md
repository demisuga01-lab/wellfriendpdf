# Table Structure Quality

This is the Transparency Rendering table-structure pass. The numbers below are indicative (approx 200-file subset), measured on the first 200 `has-tables` files from `test_corpus/` with Python JSON ground-truth loading, subprocess isolation, `--max-workers 4`, 60s timeout, and 2048 MB memory cap. They are not final benchmark claims; Multilingual Color Glyphs owns the full validation run.

## Summary

Wellfriend's table quality improved from a high-recall/low-precision shape to a more balanced extractor on the indicative 200-file has-tables subset. Table cell-F1 moved from 0.858 to 0.936, precision from 0.808 to 0.896, recall from 0.958 to 0.997, and TEDS-approx from 0.669 to 0.893. The dominant issue was over-detection: boxed prose/cards, sparse ruled regions, and repeated page furniture were being emitted as tables, while real tables often included title or note rows that damaged structure scoring. The fix tightens candidate acceptance, trims sparse title/furniture rows, and rejects prose-like borderless grids without changing the scorer.

## Provenance

| item | value |
| --- | --- |
| run date | 2026-06-26 local / 2026-06-25 UTC |
| corpus | `test_corpus/`, `has-tables`, first 200 files by deterministic harness order |
| harness | `extraction-benchmark/scripts/competitive_benchmark.py` |
| task | `tables` |
| command | `python extraction-benchmark/scripts/competitive_benchmark.py --corpus E:/wellpdfsdk/test_corpus --output-dir target/roadmap-transparency_rendering/after1 --report target/roadmap-transparency_rendering/after1.md --wellfriendpdf-bin target/debug/wellfriendpdf.exe --category has-tables --limit 200 --tasks tables --tools wellfriendpdf,pymupdf,pdfplumber --max-workers 4 --timeout 60 --max-memory-mb 2048 --checkpoint-every 25` |
| timeout | 60s per subprocess |
| memory cap | 2048 MB |
| concurrency | 4 workers |
| Wellfriend | `wellfriendpdf 0.1.0` |
| PyMuPDF | `1.27.2.3` |
| pdfplumber | `0.11.9` |

## Before And After

| run | tool | scored | cell-F1 | recall | precision | TEDS-approx |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| baseline | wellfriendpdf | 200 | 0.858 | 0.958 | 0.808 | 0.669 |
| after | wellfriendpdf | 200 | 0.936 | 0.997 | 0.896 | 0.893 |
| after reference | PyMuPDF | 200 | 0.846 | 0.840 | 0.854 | 0.867 |
| after reference | pdfplumber | 200 | 0.851 | 0.854 | 0.848 | 0.863 |

## Diagnosis

Precision and structure had separate but related causes.

False-table precision failures:

| file | baseline behavior | after behavior |
| --- | --- | --- |
| `pdf_000400` | 1 truth table, 19 predicted tables. Ruled cards and sparse page furniture were emitted as tables around the real table. | 11 predicted tables remain, but the real table is cleaner; precision improved from 0.319 to 0.371 and TEDS from 0.363 to 0.448. Residual over-detection remains on this file. |
| `pdf_000287` | 1 truth table, 12 predicted tables. Borderless prose/recipe-like grids and form-like regions were false positives. | 6 predicted tables; precision improved from 0.174 to 0.514 and TEDS from 0.261 to 0.580. |
| `pdf_000422` | 1 truth table, 3 predicted tables, but the matched structure scored 0. | 2 predicted tables; precision improved to 0.928, recall to 1.000, and TEDS to 0.889. |

Structural TEDS failures:

| pattern | effect | fix |
| --- | --- | --- |
| Sparse title/note rows before the header | Real tables had extra first rows, so predicted row counts and header position diverged from truth. | Trim sparse edge rows before scoring a candidate as a table. |
| One-column or nearly empty ruled boxes | Cards and forms were treated as table grids, adding false cells and false tables. | Require at least two populated columns, multiple populated rows, and enough non-empty cells. |
| Two-column prose aligned like a grid | Borderless detector treated ordinary prose columns as tables. | Reject prose-heavy borderless grids with low numeric/code evidence. |

## Implementation

The table analyzer now post-processes both ruled and borderless candidates before emitting them:

- `trim_sparse_table_edges` removes sparse leading/trailing title, note, or furniture rows from otherwise grid-like tables.
- `has_table_shape_evidence` rejects weak candidates with too few rows, columns, populated rows, populated columns, or non-empty cells.
- `header_like_row` requires the first surviving row to look like a compact header row rather than prose.
- `looks_like_prose_grid` rejects borderless candidates dominated by sentence-like prose without table-like numeric/code cells.

This is detection/structure logic only. The scorer was not loosened.

## Residual Gap

Wellfriend now beats PyMuPDF and pdfplumber on this indicative subset for cell-F1, precision, recall, and TEDS-approx, but over-detection is not eliminated. Files such as `pdf_000400`, `pdf_000191`, and `pdf_000106` still contain residual false table fragments, mostly from complex ruled page furniture that has enough grid evidence to pass the stricter filter. Multilingual Color Glyphs should validate whether this pattern persists on the full corpus before adding heavier page-region merging or semantic filtering.

## Regression Coverage

Added focused engine tests for:

- ruled tables with sparse title/furniture rows before the real header;
- ruled prose/card boxes that must not become tables;
- borderless two-column prose that must remain text.

Additional validation:

| check | result |
| --- | --- |
| table subset benchmark | Wellfriend after metrics: cell-F1 0.936, precision 0.896, recall 0.997, TEDS-approx 0.893 |
| text/field smoke on same 200 has-tables files | 200 text records scored, 155 field records scored, no subprocess failures |
| robustness smoke | Wellfriend survived 200/200 indicative robustness files; 87.5% parsed text artifacts, 25 clean handled errors |

