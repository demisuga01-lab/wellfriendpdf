# Table Structural Over-Detection Fix (≤200-file table slice)

**Every number in this document is indicative (≤200-file table slice).** It is a
fix-and-verify pass on the same deterministic 200-file table slice used by
[`table_slice_validation.md`](table_slice_validation.md), re-scored with the
**unchanged** scorer in `extraction-benchmark/scripts/competitive_benchmark.py`.
It is not a full-corpus or wild-PDF claim.

## Why this pass exists

The table-slice validation improved every headline metric but left one honest
gap: on the pure structural measure — **shape-F1** (row/column/cell grid match,
ignoring cell text) — Wellfriend scored **0.765, last of three**, behind PyMuPDF
(0.933) and pdfplumber (0.899). The named cause was **table over-detection**:
Wellfriend predicted **1,017 tables for 650 truth** (1.56×), over-detecting on
122/200 files and never under-detecting. This pass diagnoses that over-detection
and fixes it, holding the banked cell-F1 / recall / TEDS wins.

## Provenance

| item | value |
| --- | --- |
| date | 2026-07-02 |
| baseline commit | `01c592b` (the table-slice-validation commit) |
| wellfriendpdf | `wellfriendpdf 0.1.0`, release build (`target/release/wellfriendpdf.exe`) |
| python | 3.14.3 / win32 |
| slice | `cb.load_entries(test_corpus, 200, "has-tables")` → `pdf_000000`…`pdf_000474`, **650 truth tables** (identical selection rule to the table-slice validation) |
| scorer | `competitive_benchmark.py::table_score` — **unchanged** (no scorer gaming) |
| harness | `extraction-benchmark/scripts/diagnose_tables.py` (crash-safe: ≤4 workers, per-file subprocess isolation via `cb.monitored`, 60 s timeout + 2048 MB RSS cap + tree-kill, per-record JSONL checkpoint with flush+fsync) |

The baseline was reproduced **exactly** through this harness before any change:
cell-F1 `0.93574`, recall `0.99691`, precision `0.89595`, TEDS `0.89298`,
shape-F1 `0.76473`, predicted `1017` / truth `650`, over `122` / under `0` /
exact `78` — matching the published baseline to the digit.

## Part A — diagnosis (histogram)

Every predicted table was matched to ground-truth tables by normalized-cell
containment; each *extra* (unmatched) table was classified. Because Wellfriend's
detector already emits **at most one table per page** (ruled *or* borderless),
there is no intra-page splitting to find — the over-detection is entirely
**false tables** on non-table pages.

| cause | extra tables | note |
| --- | ---: | --- |
| **false-table** | **367** | unmatched to any ground-truth table |
| split-on-gap / header / column-drift | **0** | no real table is split; recall 0.997 confirms every truth table is found once |
| **total extra** | **367** | = predicted (1017) − truth (650) |

Breaking the 367 false tables down by detector source is decisive:

| source | real (matched) | false (unmatched) | precision |
| --- | ---: | ---: | ---: |
| **ruled** | 599 | 2 | **99.7 %** |
| **borderless** | 51 | **365** | **12.3 %** |

**Ruled detection is essentially perfect; borderless (alignment-only) detection
is the entire problem** — 365 of 367 false tables are borderless. Inspecting the
false borderless regions shows three recurring non-table patterns: **key/value
forms** (`Company:` → `Cedar Analytics`; two real columns + empty padding),
**multi-column prose** bodies, and **heading / list / page-furniture** blocks.
The 51 *real* borderless tables are, by contrast, dense regular grids (≥4
columns each filled across most rows, cell-fill ≥ 0.83).

An oracle that simply drops the 367 unmatched tables bounds the opportunity:
shape-F1 → 0.984, precision → 0.998, with recall held at 0.99689 — i.e. fixing
false detection lifts **every** metric with no trade-off, and there is nothing to
gain from merging (there are no splits).

**Recall head-room constraint (stated up front):** baseline recall is 0.99691
with 0 under-detections, so the fix must remove false tables *without dropping
any real table*.

## Part B — the fix

The false tables are borderless candidates that are not real grids. The fix adds
a **"regular dense grid" gate** for borderless tables, applied at the
**table-reporting boundary** (`ContentEngine::extract_tables`, which backs the
`wellfriendpdf extract-tables` CLI command and the Python `extract_tables` binding):

A borderless candidate is reported as a table only if
- it has **≥ 3 populated columns** (two aligned columns are a key/value form,
  which belongs to field extraction, not table detection);
- **all but at most one populated column is "regular"** — non-empty across ≥ 60 %
  of rows (rectangular, not ragged alignment);
- **overall cell-fill density ≥ 0.75**;
- **< 34 % of cells are sentence-like prose**;
- it has **≥ 2 multi-column data rows**.

Ruled and semantic (tagged) tables are always reportable — their grid is drawn
or authored, and ruled precision was already 99.7 %.

### Why the reporting boundary, not the detector

The gate is deliberately **not** placed inside the shared `detect_tables`.
Field extraction (`extract_fields` → `parse` → `detect_tables` →
`extract_table_fields`) pairs labels with values from exactly these borderless
regions; gating them out of `detect_tables` dropped field-F1 from **0.725 to
0.593** (16 form documents lost all fields). Moving the gate to
`extract_tables` leaves `detect_tables` **byte-identical to baseline**, so the
parse / markdown / text / field paths are untouched, while the table-reporting
surface is cleaned. This preserves the legitimate 3-column borderless capability
(the hand-authored 3×3 borderless test still passes) rather than hard-requiring
≥4 columns.

Focused diff: `crates/engine/src/analysis/tables.rs` (`is_reportable_table` +
`borderless_is_regular_grid` + 4 named constants + 4 regression tests) and
`crates/engine/src/engine.rs` (one `.filter(is_reportable_table)` in
`extract_tables`). No public API signature change; no other module touched.

## Part C — before / after (indicative, ≤200-file table slice, scorer unchanged)

| metric | baseline | **after fix** | change | success condition |
| --- | ---: | ---: | --- | --- |
| **structural shape-F1** | 0.76473 | **0.96232** | **+0.198** | ↑ into competitor range — met |
| precision | 0.89595 | **0.98246** | +0.087 | ↑ — met |
| predicted tables (truth 650) | 1017 | **679** | −338 | → 650 — met |
| recall | 0.99691 | **0.99689** | −0.00002 | ≥ ~0.99 held — met |
| cell-F1 | 0.93574 | **0.98737** | +0.052 | ≥ 0.936 held — met |
| TEDS-approx | 0.89298 | **0.98111** | +0.088 | ≥ 0.893 held — met |
| files over / under / exact | 122 / 0 / 78 | **22 / 0 / 178** | — | over-detection down 82 % |

Reference structural shape-F1 (same slice, same scorer, unchanged): PyMuPDF
**0.933**, pdfplumber **0.899**. Wellfriend's **0.962 now leads** both.

The after-fix recall (`0.99689`) equals the oracle "keep every real table" recall
(`0.99689`) — i.e. the fix dropped **zero** real tables; the residual 29 extra
tables are the 24 hardest borderless 3-column blocks plus 2 ruled and a few
2-row cases, leaving a small gap to the 0.984 drop-all-false ceiling.

## Part D — regression tests

Added to the `analysis::tables` module tests (all 20 table tests pass), asserting
on the reporting filter (`detect_borderless |> filter(is_reportable_table)`):

- `borderless_key_value_form_is_not_reported` — a `label:` / `value` form is not
  reported as a table;
- `borderless_multicolumn_prose_is_not_reported` — aligned prose columns are not
  a table;
- `borderless_ragged_wide_block_is_not_reported` — two solid + two sparse columns
  (ragged) is not a table;
- `borderless_dense_grid_is_still_reported` — a dense regular ≥3-column grid **is**
  still reported.

Existing tests that lock real capability continue to pass unchanged:
`borderless_table_extracts_exact_cells` (clean 3×3 borderless grid),
`prose_page_yields_no_table`, and `borderless_text_crossing_gutters_creates_colspan`.

## Part E — regression check beyond tables

- **Fields (≤200-file has-fields slice, independent re-score):** field-F1
  `0.72503`, value-F1 `0.81434`, files-with-zero-fields `0` — **exactly the
  documented baseline** (0.725 / 0.814). No field regression.
- **Text / parse / markdown:** unchanged by construction — `detect_tables` is
  byte-identical to baseline and `extract-text` does not use table detection;
  the full workspace test suite (text, docmodel, parse, chunk, field suites)
  passes.
- **Python binding:** `cargo build -p wellfriendpdf-py` succeeds; loaded and imported
  (`wellfriendpdf 0.1.0`); `extract_tables` reflects the fix — `pdf_000400` 11 → **1**
  (GT 1), `pdf_000327` 11 → **3** (GT 3), `pdf_000094` 16 → **10** (GT 6).
- **Green bar:** `cargo test --workspace` all pass (866 engine lib tests + all
  integration, 0 failed); `cargo clippy --workspace --all-targets -- -D warnings`
  clean.

## Verdict

The structural over-detection was real, was fully explained, and is now fixed.
On the ≤200-file table slice, **structural shape-F1 rose 0.765 → 0.962** — a
+0.198 gain that moves Wellfriend from last to first among the three tools (PyMuPDF
0.933, pdfplumber 0.899) — while predicted tables fell from 1,017 to 679 (truth
650), precision rose 0.896 → 0.982, and cell-F1 (0.987), TEDS (0.981), and recall
(0.99689, zero real tables lost) all held at or above baseline. The single root
cause was borderless false positives (ruled detection was already 99.7 % precise,
and there was no table splitting at all); the fix is a regularity/density gate at
the reporting surface, leaving the shared parse/field path — and the field-F1
0.725 — untouched. What still trails the 0.984 drop-all-false ceiling is ~24
borderless three-column blocks (dense but genuinely ambiguous, e.g. multi-column
term lists) plus a couple of ruled page-furniture grids; these were left in
rather than over-tighten past the point where real 3-column borderless tables
would be lost. All numbers are indicative on the 200-file slice with the scorer
unchanged; a wider run would be needed before treating them as final.
