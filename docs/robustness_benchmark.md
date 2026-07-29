# Robustness Benchmark: Wild-PDF Survival

**Plain-language summary.** On this indicative (approx 200-file subset) robustness run, Wellfriend survived 100.0% of attempted files and produced parsed text artifacts for 87.5%. The main Binding Parity targets are: corrupt xref/trailer recovery (18), encryption edge (7). Clean handled errors are separated from crashes/timeouts/OOMs because a clean rejection is acceptable for malformed input, while a hard failure is not.

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
| mozilla_pdfjs_raw | reachable during Binding Surface HEAD probe |
| veraPDF_corpus | reachable during Binding Surface HEAD probe |
| govdocs1_zip | reachable during Binding Surface HEAD probe but not downloaded because first zip is about 486 MB |
| local_public_benchmark | used when present; corpus PDFs are gitignored |

## Provenance

| item | value |
| --- | --- |
| generated | 2026-06-26T15:37:17.077959+00:00 |
| commit | 24d13f597eea63e1c39f73a3a1f7b986f4754a29 |
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
| markitdown | available, not run | 0.1.6 | MIT |
| wellfriendpdf | yes | wellfriendpdf 0.1.0 | MIT |
| pdf_wellfriendpdf | yes | 0.3.68 | MIT |
| pdfminer.six | available, not run | 20251230 | MIT |
| pdfplumber | available, not run | 0.11.9 | MIT |
| pdftext | available, not run | 0.6.3 | Apache-2.0 |
| poppler | yes | pdftotext version 26.02.0 | GPL-2.0-or-later |
| pymupdf | yes | 1.27.2.3 | AGPL-3.0/commercial |
| pymupdf4llm | available, not run | 1.27.2.3 | AGPL-3.0/commercial |
| pypdf | available, not run | 6.14.2 | BSD-3-Clause |
| pypdfium2 | yes | 4.30.0 | Apache-2.0/BSD-3-Clause |

## Ranked Robustness Table

Rates below use the indicative (approx 200-file subset). Survival = PASS + CLEAN_ERROR. Parsed pass = PASS only.

| rank | tool | survival % | parsed pass % | parsed | clean errors | hard failures | crash | timeout | OOM | mean s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | pdf_wellfriendpdf | 100.0 | 90.5 | 181 | 19 | 0 | 0 | 0 | 0 | 0.2398 |
| 2 | pymupdf | 100.0 | 90.5 | 181 | 19 | 0 | 0 | 0 | 0 | 0.23106 |
| 3 | wellfriendpdf | 100.0 | 87.5 | 175 | 25 | 0 | 0 | 0 | 0 | 0.19964 |
| 4 | pypdfium2 | 100.0 | 87.0 | 174 | 26 | 0 | 0 | 0 | 0 | 0.33785 |
| 5 | poppler | 99.5 | 87.0 | 174 | 25 | 1 | 0 | 1 | 0 | 0.42683 |

Leader comparison set: pdf_wellfriendpdf 100.0%, pymupdf 100.0%, pypdfium2 100.0%, poppler 99.5%.

## Wellfriend Hard-Fails But A Competitor Survives

No Wellfriend crash/timeout/OOM/missing-output hard failures had a competitor survival on this run.

## Wellfriend Clean-Errors But A Competitor Parses

These are not crash bugs, but they are best-effort recovery gaps for Binding Parity if the category is common.

| file | tag | root cause | competitors parsed | Wellfriend error |
| --- | --- | --- | --- | --- |
| tests/corpus/pdfs/pdfjs/issue15893_reduced.pdf | encryption-edge | encryption edge | pdf_wellfriendpdf | wellfriendpdf: parse/format error: encrypted PDF: PDF is password-protected; provide the correct password  |
| tests/corpus/pdfs/pdfjs/print_protection.pdf | encryption-edge | encryption edge | pymupdf | wellfriendpdf: parse/format error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum4_unicode-test-U2F874-correct-d30412e54245.pdf | font-encoding | encryption edge | pdf_wellfriendpdf | wellfriendpdf: parse/format error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum4_unicode-test-U2F874-wrong-b5ff24d38f77.pdf | font-encoding | encryption edge | pdf_wellfriendpdf | wellfriendpdf: parse/format error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum5_unicode-corrigendum5-fixed-70421a50fd5e.pdf | font-encoding | encryption edge | pdf_wellfriendpdf | wellfriendpdf: parse/format error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum5_unicode-corrigendum5-unicode32-once-628f77af77af.pdf | font-encoding | encryption edge | pdf_wellfriendpdf | wellfriendpdf: parse/format error: encrypted PDF: PDF is password-protected; provide the correct password  |
| public-benchmark/corpus/pdfs/darpa-safedocs/darpa-safedocs_Unicode_passwords_corrigendum5_unicode-corrigendum5-unicode32-twice-a8224fe8278f.pdf | font-encoding | encryption edge | pdf_wellfriendpdf | wellfriendpdf: parse/format error: encrypted PDF: PDF is password-protected; provide the correct password  |
| renderer-benchmark/corpus/hostile/hostile_006_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | wellfriendpdf: parse/format error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_016_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | wellfriendpdf: parse/format error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_026_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | wellfriendpdf: parse/format error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_036_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | wellfriendpdf: parse/format error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_046_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | wellfriendpdf: parse/format error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |
| renderer-benchmark/corpus/hostile/hostile_056_huge-length.pdf | pathological | corrupt xref/trailer recovery | pymupdf | wellfriendpdf: parse/format error: malformed PDF: xref stream object 1 0 is not /Type /XRef  |

## Wellfriend Root-Cause Grouping

Clean errors are included here but are distinguished from hard failures.
| category | all Wellfriend non-pass files | hard-failure subset |
| --- | --- | --- |
| corrupt xref/trailer recovery | 18 | 0 |
| encryption edge | 7 | 0 |

## Files No Tool Parsed

| file | tag |
| --- | --- |
| renderer-benchmark/corpus/hostile/hostile_000_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_001_truncated.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_010_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_011_truncated.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_020_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_021_truncated.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_030_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_031_truncated.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_040_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_041_truncated.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_050_random.pdf | pathological |
| renderer-benchmark/corpus/hostile/hostile_051_truncated.pdf | pathological |

## Prioritized Fix List For Binding Parity

1. **corrupt xref/trailer recovery** (18 files): Improve best-effort xref/trailer recovery and object scanning.
2. **encryption edge** (7 files): Harden encrypted-file detection and unsupported security-handler errors.

## Still Unmeasured

This run is capped at 200 PDFs and is text-extraction-only. It does not score text correctness, does not include a separate image/rendering robustness pass, and does not replace sustained wild-PDF exposure over time.

## Final Capped Verdict

Wellfriend had 100.0% survival on this 200-PDF robustness corpus: no crashes, panics, timeouts, or OOMs. Its parsed-pass rate was 87.5%, behind pdf_wellfriendpdf and PyMuPDF at 90.5%, because Wellfriend returned clean errors on more files. That is robust behavior for malformed input, but it is still a best-effort recovery gap.

## Prioritized Backlog

1. **Corrupt xref/trailer recovery**: 18 Wellfriend clean-error files; competitors parsed some of them.
2. **Encryption edge cases**: 7 Wellfriend clean-error files; keep failures clean, then improve empty-password/handler handling where safe.
3. **Broader long-run evidence**: maintain the no-crash property over time and add external security review before any enterprise-grade certification claim.

