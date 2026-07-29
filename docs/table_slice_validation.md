# Table-Slice Validation Benchmark (≤200-file table slice)

**Every number in this document is indicative (≤200-file table slice).** It is a
fast-loop validation of structured-table extraction on a deterministic 200-file
subset, not a full-corpus or wild-PDF claim.

## Why this run exists

The 200-PDF capped scorecard in [`competitive_benchmark.md`](competitive_benchmark.md)
did not score tables — its image-heavy slice had no table ground truth. So the
table problems from the original benchmark were unverified: Wellfriend led cell-F1
(`0.857`) but had weak **precision** (`0.806` — false-positive tables) and weak
**structure** (TEDS-approx `0.667`, behind pdfplumber `0.863` and PyMuPDF
`0.868`). Transparency Rendering was meant to raise precision and TEDS without crushing recall.
This run measures whether it did. Nothing else is re-tested here.

## Provenance

| item | value |
| --- | --- |
| date | 2026-06-27 |
| commit | `01c592b718f17513083d32556c5b206e9159454d` |
| wellfriendpdf | `wellfriendpdf 0.1.0` (release build, `target/release/wellfriendpdf.exe`) |
| python | 3.14.3 |
| platform | win32 |
| harness | `extraction-benchmark/scripts/competitive_benchmark.py` |
| run args | `--category has-tables --limit 200 --tasks tables --tools wellfriendpdf,pdfplumber,pymupdf --max-workers 4 --timeout 60 --max-memory-mb 2048` |
| concurrency | ≤4 workers, per-(tool,file) subprocess isolation |
| safety | 60 s timeout + 2048 MB RSS cap per child, `taskkill /T /F` tree-kill, per-record flush+fsync checkpoint, `WinError 1450` detection with checkpoint-and-stop |
| pass definition | subprocess exits 0 before timeout/memory cap and writes the expected JSON artifact |
| raw artifacts | `target/competitive-benchmark/tableslice-validation/{summary.json,records.jsonl}` |

The run completed 200/200 files for all three tools with **0 crashes, 0
timeouts, 0 resource errors** (the earlier `WinError 1450` did not recur).

## The slice (deterministic, ≤200)

| item | value |
| --- | --- |
| total ground-truth JSON files in `test_corpus/` | 5000 |
| files with a non-empty `tables` array | 1973 |
| files selected (first-200 by sorted filename) | **200** (`pdf_000000` … `pdf_000474`) |
| ground-truth tables in slice | 650 |
| ground-truth (non-empty) cells in slice | 105,829 |

Selection mirrors the harness exactly: glob `*.json` sorted by filename, keep
files whose label has a truthy `tables` array, take the first 200. Ground-truth
JSON is loaded with Python `json`. 1973 ≥ 200, so the slice is exactly 200 — no
padding with non-table files.

## Scoring method (identical for every tool — no scorer gaming)

The same `table_score()` scores all tools. Ground-truth `headers`+`rows` and each
tool's predicted rows are reduced to multisets of normalized non-empty cell
strings (lowercased, whitespace-collapsed); false-positive cells/tables count
against precision.

- **cell-F1 / recall / precision**: multiset overlap of cell text.
- **shape-F1**: multiset overlap of per-table grid shapes (`rows × cols`) —
  pure structural fidelity, independent of cell text.
- **TEDS-approx**: `0.75 × cell-F1 + 0.25 × shape-F1` (composite content+structure).
- All metrics are computed per file, then **macro-averaged** across the 200
  files. The baseline below was produced the same way through the same harness,
  so the before/after is apples-to-apples.

## Results — ranked by cell-F1 (indicative, ≤200-file table slice)

| rank | tool | scored | cell-F1 | recall | precision | TEDS-approx | shape-F1 | pass % | mean s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | **wellfriendpdf** | 200 | **0.936** | **0.997** | **0.896** | **0.893** | 0.765 | 100.0 | 0.13 |
| 2 | pdfplumber | 200 | 0.851 | 0.854 | 0.848 | 0.863 | 0.899 | 100.0 | 0.94 |
| 3 | pymupdf | 200 | 0.846 | 0.840 | 0.854 | 0.867 | **0.933** | 100.0 | 0.92 |

All four headline metrics (cell-F1, recall, precision, TEDS-approx) lead. The one
column Wellfriend does **not** lead is the pure structural **shape-F1** (0.765 vs
pymupdf 0.933, pdfplumber 0.899) — see the honest read below.

Capability gaps (not scored as a silent 0):

| tool | status | reason |
| --- | --- | --- |
| docling | NOT-RUN | Not importable in the benchmark venv (`ModuleNotFoundError: No module named 'docling'`). No number fabricated. |
| pdf_wellfriendpdf 0.3.67 | not wired for tables | Installed and advertises table extraction, but this harness has no pdf_wellfriendpdf table adapter; not scored rather than scored 0. |

## Before / after vs the baseline (same slice, harness, scorer)

| metric | baseline (before) | this run (after) | change | transparency-rendering target |
| --- | ---: | ---: | --- | --- |
| cell-F1 | 0.857 | **0.936** | +0.079 | hold (not regress) — held & rose |
| precision | 0.806 | **0.896** | +0.090 | **↑** — met |
| recall | 0.960 | **0.997** | +0.037 | hold (not regress) — held & rose |
| TEDS-approx | 0.667 | **0.893** | +0.226 | **↑** — met |

**Did Transparency Rendering's table work land? Yes.** The success condition was precision↑ and
TEDS↑ with recall and cell-F1 not regressing. **All four moved up and none
regressed.** Wellfriend went from *trailing* on TEDS-approx (0.667, behind pdfplumber
0.863 and PyMuPDF 0.868) to *leading* it (0.893). The competitor TEDS numbers are
essentially unchanged from the baseline (pdfplumber 0.863 → 0.863, PyMuPDF 0.868 →
0.867), which confirms the scorer was not loosened — the movement is in WellfriendPdf.

## Honest read — dominant remaining issue

Headline metrics improved decisively, but two honest caveats remain, both pointing
at the **same root cause: table over-detection / over-segmentation.**

1. **Structural shape-F1 still trails.** The TEDS-approx lead (0.893) is 75%
   cell-content-weighted. On the pure structural component, Wellfriend's shape-F1 is
   **0.765 — last of the three**, behind PyMuPDF (0.933) and pdfplumber (0.899).
   Wellfriend recovers cell *content* almost perfectly (recall 0.997) but reconstructs
   the *grid shape* less faithfully.

2. **Over-detection is the precision and shape drag.** Across the 200 files Wellfriend
   emits **1,017 predicted tables for 650 ground-truth tables (1.56×)**,
   over-detecting on **122/200** files and **under-detecting on 0**. It predicts
   114,166 cells vs 105,829 truth (+7.9%). By contrast PyMuPDF predicts exactly
   650 tables (exact count on all 200 files, hence its top shape-F1) and
   pdfplumber 821. So Wellfriend's residual weakness is **splitting/over-emitting
   tables**, not missing content — precision (0.896) is its lowest headline metric
   and the structural shape gap follows from the same over-segmentation.

Net: Transparency Rendering turned the original precision/TEDS weakness into a lead on every
composite metric, with recall now near-perfect. The honest remaining work is
**structural**: stop over-segmenting tables so the predicted table/cell counts and
grid shapes match ground truth, which would lift precision and shape-F1 toward the
content scores.

## Reproduce

```sh
# 1) Table-only run on the deterministic first-200 has-tables slice (crash-safe).
.\.venv-public-benchmark\Scripts\python.exe extraction-benchmark\scripts\competitive_benchmark.py `
  --corpus E:\wellpdfsdk\test_corpus --category has-tables --limit 200 `
  --tasks tables --tools wellfriendpdf,pdfplumber,pymupdf `
  --output-dir target\competitive-benchmark\tableslice-validation `
  --report target\competitive-benchmark\tableslice-validation.md `
  --max-workers 4 --timeout 60 --max-memory-mb 2048

# 2) Independent verification: recompute from records.jsonl, structural
#    aggregates, AND an independent wellfriendpdf re-run re-scored with the same scorer.
.\.venv-public-benchmark\Scripts\python.exe extraction-benchmark\scripts\verify_tableslice.py
```

Workspace green at this commit: `cargo clippy --workspace --all-targets -- -D
warnings` and `cargo test --workspace` both pass. No engine source was changed for
this measurement run.
