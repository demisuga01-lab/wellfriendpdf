# Field Extraction Benchmark

Date: 2026-06-25

This Release Packaging run improved Wellfriend's strict field-F1 on the deterministic first 200
`has-fields` files from `0.1035` to `0.6635` (indicative, ~200-file subset).
The dominant failure was not AcroForm parsing: the corpus fields are mostly
rendered key-value text recovered as paragraphs or tables. Wellfriend was missing
common labels, pairing label cells with neighboring labels, and failing to split
collapsed cells containing several `Label: value` pairs.

## Corpus And Scoring

- Corpus: first 200 files from `test_corpus/` whose JSON labels contain
  `fields`, selected deterministically by sorted filename.
- Ground truth schema: JSON `fields` object mapping key text to expected value
  text.
- Strict field score: key and value must both match after benchmark
  normalization.
- Value-only score: value match ignoring key, useful for identifying label-key
  mapping misses.
- These numbers are indicative for the 200-file fast loop, not final full-corpus
  results.

## Diagnosis

Sample inspection showed the misses clustered into four buckets:

- Key detection: common labels such as `Account`, `Period`, `Closing Balance`,
  `Passenger`, `Seat`, `Gate`, `Booking`, `Department`, `Requested By`, and
  `Account ID` were absent from the spatial label lexicon.
- Association: table-recovered forms with a label row followed by a value row
  were not paired by column. That produced misses, and sometimes bad pairs such
  as `DATE -> SEAT`.
- Collapsed cells: long recovered table cells such as
  `Account: AC-... Period: ... Closing Balance: ...` were split only at the first
  colon, so most fields were lost.
- Schema mapping: many remaining values are now found under a semantically
  related key, especially benchmark `due` versus Wellfriend `due_date`, and `total`
  versus `total_due`.

## Changes

- Expanded spatial label detection for the high-frequency rendered-form labels.
- Added multi-pair inline extraction from collapsed text/table cells.
- Added compact table label-row/value-row pairing for small key-value grids.
- Added boarding-pass compact-grid extraction for one-cell recovered layouts.
- Restricted same-row table pairing to simple two-cell rows to avoid treating
  regular data-table headers as fields.
- Tightened invoice profile matching so one-word synonyms such as `date` no
  longer steal more specific labels such as `Due Date`.
- Added regression tests for multi-pair splitting, table row pairing, compact
  boarding-pass extraction, and profile label matching.

## Results

| metric | before | after |
| --- | ---: | ---: |
| scored files | 200 | 200 |
| strict field-F1 | 0.1035 | 0.6635 |
| field recall | 0.0842 | 0.7806 |
| field precision | 0.1649 | 0.6288 |
| value-only F1 | 0.1182 | 0.7480 |

The improvement is a large multiple of the starting score, and both precision
and recall moved up together.

## Remaining Limits

- `due` remains the largest strict-key miss: all 96 missed `due` values were
  present under `due_date`. Renaming or aliasing that public key would improve
  the benchmark score, but it changes the public field schema and was left for a
  deliberate API decision.
- Some `total` values are present as `total_due`; this is another schema-alias
  issue rather than pure value extraction failure.
- Several one-page boarding-pass files still parse to zero fields when the
  document model does not expose enough separable structure.
- Phone/priority misses remain mostly in generic-form pages and need a second
  tuning pass if those fields become product-critical.

## Validation

- `cargo test -p wellfriendpdf-engine spatial::tests -- --nocapture`
- `cargo test -p wellfriendpdf-engine profile::tests -- --nocapture`
- `cargo test -p wellfriendpdf-engine --test extract_fields -- --nocapture`
- `cargo build --release -p wellfriendpdf-cli`
- Field benchmark:
  `python extraction-benchmark\scripts\competitive_benchmark.py --corpus E:\wellpdfsdk\test_corpus --category has-fields --limit 200 --tools wellfriendpdf --tasks fields --output-dir target\competitive-benchmark\release_packaging-fields-after --report target\competitive-benchmark\release_packaging-fields-after.md --max-workers 4 --timeout 60 --max-memory-mb 2048`
- Clean first-200 text/table check: text word-F1 `1.0000`, line recall
  `1.0000`; no table-scored files in that slice.
- Has-tables 200-file check: table cell-F1 `0.85754`, recall `0.95775`,
  precision `0.80815`, TEDS approx `0.66906`.
