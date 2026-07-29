# Field-Slice Validation Benchmark (≤200-file field slice)

**Every number in this document is indicative (≤200-file field slice).** It is a
fast-loop validation of key-value field extraction on a deterministic 200-file
subset, not a full-corpus or wild-PDF claim.

## Why this run exists

The 200-PDF capped scorecard in [`competitive_benchmark.md`](competitive_benchmark.md)
validated text, speed, and image count, but it used an image-heavy slice that
contained **no** field ground truth, so field extraction was not scored there.
The campaign's single worst measured number — strict field-F1 `0.104`
(precision `0.166`, recall `0.085`) at baseline — was therefore left unverified.
This run scores field extraction directly and reports honestly whether the
Release Packaging field work landed. Nothing else is re-tested here.

## Provenance

| item | value |
| --- | --- |
| date | 2026-06-27 |
| commit | `01c592b718f17513083d32556c5b206e9159454d` |
| wellfriendpdf | `wellfriendpdf 0.1.0` (release build, `target/release/wellfriendpdf.exe`) |
| python | 3.14.3 |
| platform | win32 |
| harness | `extraction-benchmark/scripts/competitive_benchmark.py` |
| run args | `--category has-fields --limit 200 --tasks fields --tools wellfriendpdf,pypdf --max-workers 4 --timeout 60 --max-memory-mb 2048` |
| concurrency | ≤4 workers, per-(tool,file) subprocess isolation |
| safety | 60 s timeout + 2048 MB RSS cap per child, `taskkill /T /F` tree-kill, per-record flush+fsync checkpoint, `WinError 1450` detection with checkpoint-and-stop |
| pass definition | subprocess exits 0 before timeout/memory cap and writes the expected JSON artifact |
| raw artifacts | `target/competitive-benchmark/fieldslice-validation/{summary.json,records.jsonl}` |

The run completed 200/200 files with **0 crashes, 0 timeouts, 0 resource
errors** (the earlier `WinError 1450` did not recur).

## The slice (deterministic, ≤200)

| item | value |
| --- | --- |
| total ground-truth JSON files in `test_corpus/` | 5000 |
| files with a non-empty `fields` object | 2030 |
| files selected (first-200 by sorted filename) | **200** (`pdf_000000` … `pdf_000466`) |
| pages in slice | 1731 (range 1–40) |
| ground-truth field (key,value) pairs in slice | 1942 |
| slice PDFs containing AcroForm form fields | **0 / 200** |

Selection mirrors the harness exactly: glob `*.json` sorted by filename, keep
files whose label has a truthy `fields` object, take the first 200. Ground-truth
JSON is loaded with Python `json` (case-sensitive `Reference`/`reference` keys).
2030 ≥ 200, so the slice is exactly 200 — no padding with non-field files.

## Scoring method (identical for every tool — no scorer gaming)

The same `field_score()` scores all tools. Keys are normalized
(lowercase, non-alphanumeric → `_`); values are normalized (lowercase,
whitespace-collapsed; structured values prefer `iso`/`text`/`amount`/`number`).

- **Strict field-F1**: a true positive requires **both key and value** to match.
- **Value-only F1**: value matches regardless of key (isolates label-naming
  misses from value-extraction misses).
- **Precision / recall**: standard, per file, then **macro-averaged** across the
  200 files (mean of per-file scores). The `0.104` baseline was computed the same
  way through the same harness, so the before/after below is apples-to-apples.

## Results — ranked (indicative, ≤200-file field slice)

| rank | tool | scored | strict field-F1 | recall | precision | value-only F1 | pass % | mean s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | **wellfriendpdf** | 200 | **0.725** | 0.845 | 0.692 | 0.814 | 100.0 | 0.160 |
| 2 | pypdf | 200 | 0.000 | 0.000 | 0.000 | 0.000 | 100.0 | 0.332 |

Tools noted as a capability gap rather than scored as a silent 0:

| tool | status | reason |
| --- | --- | --- |
| pypdf 6.14.2 | ran, scored 0.0 | AcroForm form-field reader (`get_fields`). **0 / 200** slice PDFs contain AcroForm widgets, so it recovers nothing. Source/capability mismatch, not a quality loss. |
| pymupdf 1.27.2.3 | capability probe only | AcroForm widget reader; probe found **0 / 200** files with widgets. Same mismatch as pypdf — not scored to avoid implying a head-to-head loss. |
| pdf_wellfriendpdf 0.3.67 | capability gap | Its "form field" support is AcroForm fill/read, not heuristic rendered-KV extraction; nothing to recover on this rendered-text corpus. |
| docling | NOT-RUN | Not importable in the benchmark venv (`ModuleNotFoundError: No module named 'docling'`). It also exposes no comparable key-value field API. No number is fabricated. |

The corpus `fields` are **rendered key-value text** (e.g. `Bill To: Atlas Office
Group`, `Account: AC-183031`), not interactive form fields. Wellfriend's
`extract-fields` is a heuristic/template rendered-KV extractor — the capability
actually under test. AcroForm-only tools are doing a different job that this
corpus gives them nothing to do.

## Before / after vs the 0.104 baseline (same slice, harness, scorer)

| metric | baseline (before) | this run (after) | change |
| --- | ---: | ---: | --- |
| strict field-F1 | 0.104 | **0.725** | +0.621 (~7.0×) |
| precision | 0.166 | **0.692** | +0.526 |
| recall | 0.085 | **0.845** | +0.760 |
| value-only F1 | 0.118¹ | **0.814** | +0.696 |

¹ value-only baseline from `docs/field_extraction_benchmark.md` (Release Packaging "before").

**Did Release Packaging's field work land? Yes.** Strict field-F1 moved from `0.104` to
`0.725` — roughly a 7× improvement — and **both precision and recall rose
together** (precision ×4.2, recall ×9.9). This is not a one-metric-up,
other-metric-down artifact. The current build measures `0.725`, at/slightly above
the `0.6635` recorded for the Release Packaging build in `field_extraction_benchmark.md`,
so the field work is intact and has not regressed.

## Honest read — dominant remaining failure modes

Strict F1 is `0.725`, not ~`1.0`. The gap is **not** value-extraction failure;
it is **key naming and over-prediction**. Evidence from a re-run that recomputed
the same metrics three ways (summary.json, recompute-from-records, and an
independent wellfriendpdf re-run — all `0.72503`):

1. **Key-alias / schema-mapping (the dominant strict miss).** The value is
   extracted correctly but filed under a different key name than the benchmark's
   canonical key. Of the missed truth keys, the value is present under another
   key in: `due`→`due_date` 95/96, `total`→`total_due` 34/35, plus all 30 `date`,
   all 25 `from`, all 25 `to` — ≥209 truth values that are correct but
   mis-keyed. This is exactly why **value-only F1 (0.814) ≫ strict F1 (0.725)**.

2. **Over-prediction (the dominant precision drag).** Wellfriend emits 3650 predicted
   pairs against 1942 truth pairs (1.88×), because its document-type profile
   emits a fixed key set per detected doc type even when a given file's ground
   truth lists only a subset. As a result the **pooled/micro precision is 0.459**,
   below the macro precision of `0.692`. Top spurious keys: `due_date` (194,
   itself an alias artifact), `account` (185), `reference` (146), `total_due`
   (100), `bill_to` (90). Both numbers are reported so the macro headline is not
   read in isolation.

3. **Pure value-detection misses are a small minority**: `account` 17, `period`
   15, `closing_balance` 6, and a long thin tail. **0 of 200** files produced
   zero fields — every document yielded extraction.

Net: the residual error is dominated by **label/schema mapping**
(`due`/`due_date`, `total`/`total_due`) and **profile over-emission**, not by an
inability to find values. Renaming the public field schema to the benchmark's
canonical keys would mechanically raise strict F1 toward value-only F1 (~0.81),
but that is a deliberate public-API decision and was not done here to flatter the
score.

## Reproduce

```sh
# 1) Field-only run on the deterministic first-200 has-fields slice (crash-safe).
.\.venv-public-benchmark\Scripts\python.exe extraction-benchmark\scripts\competitive_benchmark.py `
  --corpus E:\wellpdfsdk\test_corpus --category has-fields --limit 200 `
  --tasks fields --tools wellfriendpdf,pypdf `
  --output-dir target\competitive-benchmark\fieldslice-validation `
  --report target\competitive-benchmark\fieldslice-validation.md `
  --max-workers 4 --timeout 60 --max-memory-mb 2048

# 2) Independent verification: recompute metrics from records.jsonl AND re-run
#    wellfriendpdf on the same 200 files for the key-level failure breakdown.
.\.venv-public-benchmark\Scripts\python.exe extraction-benchmark\scripts\verify_fieldslice.py

# 3) Capability proof: confirm the slice has no AcroForm widgets.
.\.venv-public-benchmark\Scripts\python.exe extraction-benchmark\scripts\capability_probe.py
```

Workspace green at this commit: `cargo clippy --workspace --all-targets -- -D
warnings` and `cargo test --workspace` both pass. No engine source was changed
for this measurement run.
