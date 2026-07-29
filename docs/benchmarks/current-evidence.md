# Current README evidence

Generated for commit `d346915de5125fccf3163847cb3ebec197c49046`.

## Evidence classification

| Class | Meaning |
| --- | --- |
| `measured_directly` | Wellfriend and comparator ran equivalent or narrowly comparable operations on the same machine/input. |
| `validated_in_repository` | Current code, tests, Prompt 36 artifacts, save/reopen checks, package gates, fuzzing, or executed oracles support the claim. |
| `official_competitor_documentation` | Current official competitor documentation describes the capability; it was not benchmarked here. |
| `inferred_limited` | A narrow inference from evidence and labeled as inference. |
| `unavailable_or_not_measured` | Tool, license, host support, corpus, or equivalent operation was unavailable. |

## Prompt 36 release evidence

| Metric | Result | Evidence class | Source artifact |
| --- | ---: | --- | --- |
| Implementation status | `complete` | validated_in_repository | `target/prompt36-enterprise-validation/final-release-verdict.json` |
| Release posture | `release_ready_with_limits` | validated_in_repository | `target/prompt36-enterprise-validation/final-release-verdict.json` |
| Closure criteria | 42 / 42 pass | validated_in_repository | `target/prompt36-enterprise-validation/final-validation-matrix.json` |
| Maximum observed RSS | 6,618,920 KiB | validated_in_repository | `target/prompt36-enterprise-validation/performance-results.json` |
| Memory budget | 33,554,432 KiB | validated_in_repository | `target/prompt36-enterprise-validation/performance-results.json` |
| Fuzz targets | 43 targets built | validated_in_repository | `target/prompt36-enterprise-validation/fuzz-results.json` |
| Fuzz smoke | 64 runs per target | validated_in_repository | `target/prompt36-enterprise-validation/fuzz-results.json` |
| Coverage JSON rerun | exit 0, 399 s, 2,963,924 KiB peak RSS | validated_in_repository | `target/prompt36-enterprise-validation/coverage-results.json` |
| Workspace check | Prompt 36 final check passed | validated_in_repository | `target/prompt36-enterprise-validation/performance-results.json` |
| qpdf and Poppler oracles | available and run | validated_in_repository | `target/prompt36-enterprise-validation/external-tool-support-matrix.json` |

## Binding evidence

Prompt 36 binding matrix:

- Rust: pass
- CLI: pass
- Python: pass
- C ABI: pass
- WASM: pass
- .NET: pass
- Java Maven: pass
- Java Gradle: exact host limit because VPS Gradle 4.4.1 cannot evaluate the modern settings file

Source: `target/prompt36-enterprise-validation/binding-release-matrix.json`.

## README direct comparator smoke

VPS result folder: `/home/demisuga01/wellpdf/results/readme-competitor-20260729T175541Z`.

Input: compact repository-owned fixture normalized through pikepdf for qpdf-compatible structural checks. This is a README smoke, not a corpus benchmark.

| Operation | Result | Duration (s) | Evidence class |
| --- | ---: | ---: | --- |
| Wellfriend CLI `extract-text` | pass | 0.220 | measured_directly |
| Wellfriend CLI `parse --format json` | pass | 0.207 | measured_directly |
| qpdf `--check` | pass | 0.006 | measured_directly |
| qpdf `--linearize` | pass | 0.008 | measured_directly |
| Poppler `pdfinfo` | pass | 0.014 | measured_directly |
| Poppler `pdftotext` | pass | 0.013 | measured_directly |
| Poppler `pdftoppm` | pass | 0.015 | measured_directly |
| pypdfium2 / PDFium render | pass | 0.356 | measured_directly |
| PyMuPDF / MuPDF text + render | pass | 0.276 | measured_directly |
| pikepdf / qpdf open + save | pass | 0.104 | measured_directly |
| pdfplumber extraction | pass | 0.143 | measured_directly |

Source: `target/readme-rewrite/readme-direct-comparisons-qpdf-clean.json`.

## Limits

- The README smoke is not a stress test, malformed corpus, commercial SDK benchmark, rendering-quality matrix, OCR accuracy study, or accessibility certification.
- Wrapper relationships are disclosed: pypdfium2 is a PDFium wrapper, PyMuPDF is a MuPDF binding, and pikepdf is a qpdf wrapper.
- Unavailable tools are not scored as competitor failures.
