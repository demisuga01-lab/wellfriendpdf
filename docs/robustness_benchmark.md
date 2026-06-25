# Robustness Benchmark: Wild-PDF Survival

**Plain-language summary.** On this indicative (approx 200-file subset) robustness run, Oxide survived 100.0% of attempted files and produced parsed text artifacts for 83.0%. The main Prompt 2 targets are: corrupt xref/trailer recovery (20), encryption edge (7), allocation from untrusted size (6). Clean handled errors are separated from crashes/timeouts/OOMs because a clean rejection is acceptable for malformed input, while a hard failure is not.

## Scope And Corpus

This is a SMALL indicative robustness corpus, not a final robustness claim. It has no ground-truth text labels, so it measures survival only.

| metric | value |
| --- | --- |
| label | indicative (approx 200-file subset) |
| files | 200 |
| selection | deterministic: all selected sources are sorted by path/category and truncated to the first 200 entries |

### Source Breakdown

| source tier | files |
| --- | --- |
| tier1-in-repo | 75 |
| tier1-in-repo-renderer | 80 |
| tier2-public-wild | 39 |
| tier3-generated-broken | 6 |

### Stress Tags

| stress tag | files |
| --- | --- |
| corrupt-xref | 1 |
| deep-nesting | 1 |
| encryption-edge | 6 |
| font-encoding | 26 |
| forms | 14 |
| garbage-after-eof | 1 |
| huge-declared-length | 1 |
| large-or-multipage | 3 |
| layout-heavy | 9 |
| missing-startxref | 1 |
| pathological | 61 |
| real-clean | 63 |
| scanned-or-image | 10 |
| truncated | 1 |
| unsupported-or-rare-filter | 2 |

### Public Source Reachability

| source | status |
| --- | --- |
| mozilla_pdfjs_raw | reachable during Prompt 1 HEAD probe |
| veraPDF_corpus | reachable during Prompt 1 HEAD probe |
| govdocs1_zip | reachable during Prompt 1 HEAD probe but not downloaded because first zip is about 486 MB |
| local_public_benchmark | used when present; corpus PDFs are gitignored |

## Provenance

| item | value |
| --- | --- |
| generated | 2026-06-25T15:51:01.682629+00:00 |
| commit | eaf5f49d9fb3f4a27a7b495d98955bc8a43019c0 |
| python | 3.14.3 |
| platform | win32 |
| hardware | Intel64 Family 6 Model 191 Stepping 2, GenuineIntel / 20 logical CPUs |
| timeout | 60s |
| memory cap | 2048 MB |
| max workers | 4 |
| pass definition | PASS exits 0 and writes an output artifact; CLEAN_ERROR is a handled non-zero error and counts as survival, not parsed output |

## Tools Run Vs Skipped

| tool | run | version | reason/license |
| --- | --- | --- | --- |
| docling | available, not run | 2.107.0 | installed but skipped in default run because it is a heavyweight ML converter; pass --include-heavy to run it |
| markitdown | yes | 0.1.6 | MIT |
| oxide | yes | oxide 0.1.0 | MIT OR Apache-2.0 |
| pdf_oxide | yes | 0.3.68 | MIT |
| pdfminer.six | yes | 20251230 | MIT |
| pdfplumber | yes | 0.11.9 | MIT |
| pdftext | yes | 0.6.3 | Apache-2.0 |
| poppler | yes | pdftotext version 26.02.0 | GPL-2.0-or-later |
| pymupdf | yes | 1.27.2.3 | AGPL-3.0/commercial |
| pymupdf4llm | yes | 1.27.2.3 | AGPL-3.0/commercial |
| pypdf | yes | 6.14.2 | BSD-3-Clause |
| pypdfium2 | yes | 4.30.0 | Apache-2.0/BSD-3-Clause |

## Ranked Robustness Table

Rates below are indicative (approx 200-file subset). Survival = PASS + CLEAN_ERROR. Parsed pass = PASS only.

| rank | tool | survival % | parsed pass % | parsed | clean errors | hard failures | crash | timeout | OOM | mean s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | pdf_oxide | 100.0 | 90.5 | 181 | 19 | 0 | 0 | 0 | 0 | 0.38536 |
| 2 | pymupdf | 100.0 | 90.5 | 181 | 19 | 0 | 0 | 0 | 0 | 0.42755 |
| 3 | markitdown | 100.0 | 87.0 | 174 | 26 | 0 | 0 | 0 | 0 | 4.0689 |
| 4 | pdftext | 100.0 | 87.0 | 174 | 26 | 0 | 0 | 0 | 0 | 1.18825 |
| 5 | pypdfium2 | 100.0 | 87.0 | 174 | 26 | 0 | 0 | 0 | 0 | 0.67022 |
| 6 | pdfminer.six | 100.0 | 84.0 | 168 | 32 | 0 | 0 | 0 | 0 | 1.12112 |
| 7 | pdfplumber | 100.0 | 84.0 | 168 | 32 | 0 | 0 | 0 | 0 | 1.55121 |
| 8 | oxide | 100.0 | 83.0 | 166 | 34 | 0 | 0 | 0 | 0 | 0.19296 |
| 9 | pypdf | 100.0 | 79.5 | 159 | 41 | 0 | 0 | 0 | 0 | 0.66551 |
| 10 | poppler | 99.5 | 87.0 | 174 | 25 | 1 | 0 | 1 | 0 | 0.48368 |
| 11 | pymupdf4llm | 99.5 | 83.0 | 166 | 33 | 1 | 0 | 1 | 0 | 4.16235 |

Leader comparison set: pdf_oxide 100.0%, pymupdf 100.0%, pypdfium2 100.0%, poppler 99.5%.

## Oxide Hard-Fails But A Competitor Survives

No Oxide crash/timeout/OOM/missing-output hard failures had a competitor survival on this run.

## Oxide Clean-Errors But A Competitor Parses

These are not crash bugs, but they are best-effort recovery gaps for Prompt 2 if the category is common.

| file | tag | root cause | competitors parsed | Oxide error |
| --- | --- | --- | --- | --- |
| robustness-benchmark/corpus/generated/deeply_nested_open_action.pdf | deep-nesting | recursion/nesting bound | markitdown, pdf_oxide, pdfminer.six, pdfplumber, pdftext, poppler, pymupdf, pymupdf4llm, pypdf, pypdfium2 | Error: parse error: object nesting exceeded depth limit 64  |
| robustness-benchmark/corpus/generated/missing_startxref_trailer.pdf | missing-startxref | corrupt xref/trailer recovery | markitdown, pdf_oxide, pdfminer.six, pdfplumber, pdftext, poppler, pymupdf, pymupdf4llm, pypdfium2 | Error: malformed PDF: missing startxref  |
| robustness-benchmark/corpus/generated/truncated_mid_file.pdf | truncated | corrupt xref/trailer recovery | pdf_oxide, pymupdf, pymupdf4llm | Error: malformed PDF: missing startxref  |
| tests/corpus/pdfs/pdfjs/issue15893_reduced.pdf | encryption-edge | encryption edge | markitdown, pdf_oxide, pdfminer.six, pdfplumber | Error: encrypted PDF: PDF is password-protected; provide the correct password  |
| tests/corpus/pdfs/pdfjs/print_protection.pdf | encryption-edge | encryption edge | pymupdf | Error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum4_unicode-test-U2F874-correct-d30412e54245.pdf | font-encoding | encryption edge | pdf_oxide | Error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum4_unicode-test-U2F874-wrong-b5ff24d38f77.pdf | font-encoding | encryption edge | pdf_oxide | Error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum5_unicode-corrigendum5-fixed-70421a50fd5e.pdf | font-encoding | encryption edge | pdf_oxide | Error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum5_unicode-corrigendum5-unicode32-once-628f77af77af.pdf | font-encoding | encryption edge | pdf_oxide | Error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum5_unicode-corrigendum5-unicode32-twice-a8224fe8278f.pdf | font-encoding | encryption edge | pdf_oxide | Error: encrypted PDF: PDF is password-protected; provide the correct password  |
| renderer-benchmark/corpus/hostile/hostile_001_truncated.pdf | pathological | corrupt xref/trailer recovery | markitdown | Error: malformed PDF: missing startxref  |
| renderer-benchmark/corpus/hostile/hostile_002_wrong-startxref.pdf | pathological | allocation from untrusted size | markitdown, pdf_oxide, pdfminer.six, pdfplumber, pdftext, poppler, pymupdf, pymupdf4llm, pypdfium2 | Error: parse error: offset 999999 is beyond input length 607  |
| renderer-benchmark/corpus/hostile/hostile_006_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | Error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_011_truncated.pdf | pathological | corrupt xref/trailer recovery | markitdown | Error: malformed PDF: missing startxref  |
| renderer-benchmark/corpus/hostile/hostile_012_wrong-startxref.pdf | pathological | allocation from untrusted size | markitdown, pdf_oxide, pdfminer.six, pdfplumber, pdftext, poppler, pymupdf, pymupdf4llm, pypdfium2 | Error: parse error: offset 999999 is beyond input length 607  |
| renderer-benchmark/corpus/hostile/hostile_016_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | Error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_021_truncated.pdf | pathological | corrupt xref/trailer recovery | markitdown | Error: malformed PDF: missing startxref  |
| renderer-benchmark/corpus/hostile/hostile_022_wrong-startxref.pdf | pathological | allocation from untrusted size | markitdown, pdf_oxide, pdfminer.six, pdfplumber, pdftext, poppler, pymupdf, pymupdf4llm, pypdfium2 | Error: parse error: offset 999999 is beyond input length 607  |
| renderer-benchmark/corpus/hostile/hostile_026_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | Error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_031_truncated.pdf | pathological | corrupt xref/trailer recovery | markitdown | Error: malformed PDF: missing startxref  |
| renderer-benchmark/corpus/hostile/hostile_032_wrong-startxref.pdf | pathological | allocation from untrusted size | markitdown, pdf_oxide, pdfminer.six, pdfplumber, pdftext, poppler, pymupdf, pymupdf4llm, pypdfium2 | Error: parse error: offset 999999 is beyond input length 607  |
| renderer-benchmark/corpus/hostile/hostile_036_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | Error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_041_truncated.pdf | pathological | corrupt xref/trailer recovery | markitdown | Error: malformed PDF: missing startxref  |
| renderer-benchmark/corpus/hostile/hostile_042_wrong-startxref.pdf | pathological | allocation from untrusted size | markitdown, pdf_oxide, pdfminer.six, pdfplumber, pdftext, poppler, pymupdf, pymupdf4llm, pypdfium2 | Error: parse error: offset 999999 is beyond input length 607  |
| renderer-benchmark/corpus/hostile/hostile_046_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | Error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_051_truncated.pdf | pathological | corrupt xref/trailer recovery | markitdown | Error: malformed PDF: missing startxref  |
| renderer-benchmark/corpus/hostile/hostile_052_wrong-startxref.pdf | pathological | allocation from untrusted size | markitdown, pdf_oxide, pdfminer.six, pdfplumber, pdftext, poppler, pymupdf, pymupdf4llm, pypdfium2 | Error: parse error: offset 999999 is beyond input length 607  |
| renderer-benchmark/corpus/hostile/hostile_056_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | Error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |

## Oxide Root-Cause Grouping

Clean errors are included here but are distinguished from hard failures.
| category | all Oxide non-pass files | hard-failure subset |
| --- | --- | --- |
| corrupt xref/trailer recovery | 20 | 0 |
| encryption edge | 7 | 0 |
| allocation from untrusted size | 6 | 0 |
| recursion/nesting bound | 1 | 0 |

## Files No Tool Parsed

| file | tag |
| --- | --- |
| renderer-benchmark/corpus/hostile/hostile_000_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_010_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_020_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_030_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_040_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_050_random.pdf | pathological |

## Prioritized Fix List For Prompt 2

1. **corrupt xref/trailer recovery** (20 files): Improve best-effort xref/trailer recovery and object scanning.
2. **encryption edge** (7 files): Harden encrypted-file detection and unsupported security-handler errors.
3. **allocation from untrusted size** (6 files): Validate declared stream lengths against actual remaining file bytes before allocating.
4. **recursion/nesting bound** (1 files): Keep recursion limits explicit and convert over-depth input into clean errors.

## Prompt 2 Follow-Up: 2026-06-25

Prompt 2 found no Oxide crashes, panics, hangs, or OOMs in the Prompt 1 baseline. The fixes therefore targeted the largest clean-error competitor-parse gaps: best-effort xref/trailer recovery and bounded deep nesting.

### Changes Made

- Added a fallback xref rebuild path in `crates/engine/src/reader.rs`: when `startxref` is missing or the referenced xref section cannot be parsed, Oxide scans `N G obj` headers, reuses the last trailer dictionary when present, and synthesizes `/Root` from a scanned `/Catalog` when the trailer is absent.
- Kept recovery bounded with `MAX_FALLBACK_XREF_OBJECTS = 200000`; files with no recoverable catalog/trailer still return clean errors.
- Raised parser syntactic nesting from 64 to 256 in `crates/engine/src/parser.rs`, preserving a hard depth limit while allowing the generated 180-level nesting probe.
- Added regression tests for missing `startxref`, wrong `startxref`, trailer-less object-scan recovery, within-limit deep nesting, and beyond-limit clean errors.

### Before/After Robustness

Indicative (approx 200-file subset), same manifest and same non-heavy tool set:

| metric | Prompt 1 baseline | Prompt 2 after |
| --- | ---: | ---: |
| Oxide survival rate | 100.0% | 100.0% |
| Oxide parsed-pass rate | 83.0% | 87.5% |
| Oxide parsed files | 166 / 200 | 175 / 200 |
| Oxide clean errors | 34 | 25 |
| Oxide hard failures | 0 | 0 |
| Oxide clean-error/competitor-parse gaps | 28 | 19 |

Recovered files:

- `robustness-benchmark/corpus/generated/deeply_nested_open_action.pdf`
- `robustness-benchmark/corpus/generated/missing_startxref_trailer.pdf`
- `robustness-benchmark/corpus/generated/truncated_mid_file.pdf`
- `renderer-benchmark/corpus/hostile/hostile_002_wrong-startxref.pdf`
- `renderer-benchmark/corpus/hostile/hostile_012_wrong-startxref.pdf`
- `renderer-benchmark/corpus/hostile/hostile_022_wrong-startxref.pdf`
- `renderer-benchmark/corpus/hostile/hostile_032_wrong-startxref.pdf`
- `renderer-benchmark/corpus/hostile/hostile_042_wrong-startxref.pdf`
- `renderer-benchmark/corpus/hostile/hostile_052_wrong-startxref.pdf`

Remaining Oxide non-pass groups after Prompt 2:

| category | files | reason not fixed now |
| --- | ---: | --- |
| corrupt xref/trailer recovery | 18 | Remaining hostile files either lack a recoverable catalog/trailer or are too truncated to produce a meaningful page tree. Oxide returns clean errors. |
| encryption edge | 7 | These are password/permissions-encrypted inputs. Prompt 2 did not weaken encryption handling to extract text without a verified password. |

### Clean-File Checks

- Synthetic first-200 text subset: Oxide text pass rate 100.0%, word F1 1.000, line recall 1.000.
- Synthetic has-tables first-200 subset: Oxide text pass rate 100.0%; table pass rate 100.0%; table cell F1 0.85754, consistent with the existing full-report table F1 of 0.857.
- Fuzz smoke: `cargo +nightly fuzz run parse_pdf -- -runs=256 -max_len=65536` completed without findings.

### Recovery Limits

The fallback is best-effort. It recovers documents where object headers can be scanned and either a trailer dictionary or a `/Catalog` object can be found. It intentionally does not fabricate a document root for random bytes, catalogless streams, encrypted files without a verified password, or files truncated before the page tree can be recovered.

## Still Unmeasured

This run is small, text-extraction-only, and indicative. It does not prove final real-world robustness, does not score text correctness, and does not include a separate image/rendering robustness pass. The larger wild run belongs in Prompt 10.
