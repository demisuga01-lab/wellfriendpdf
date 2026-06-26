# Competitive Benchmark: Oxide vs Major PDF Tools

## 200-PDF Capped Scorecard

This scorecard is intentionally capped at 200 PDFs. Treat every number below as a 200-PDF validation result, not a broader corpus claim.

## Synthetic Corpus Caveat Up Front

This run uses 200 synthetic procedurally generated PDFs with paired JSON ground truth. It measures speed and correctness against known labels, but it is not a wild-PDF robustness benchmark. A high pass rate here does not prove robustness against malformed, scanned, handwritten, camera-captured, or adversarial PDFs.

## Provenance

| item | value |
| --- | --- |
| generated | 2026-06-26T15:39:09.424726+00:00 |
| commit | 24d13f597eea63e1c39f73a3a1f7b986f4754a29 |
| python | 3.14.3 |
| platform | win32 |
| timeout | 60s |
| memory cap | 2048 MB |
| pass definition | subprocess exits 0 before timeout/memory cap and writes the expected output artifact |

## Corpus Breakdown

| metric | value |
| --- | --- |
| files | 200 |
| pages | 561 |
| page range | 1 to 16 |
| ground-truth images | 2633 |

| tag/category | files |
| --- | --- |
| has-images | 200 |
| image-heavy | 200 |
| image-heavy-count | 70 |
| large-file | 65 |
| medium-doc | 44 |
| short-doc | 156 |

## Tools Run vs Skipped

| tool | run | version | reason/license |
| --- | --- | --- | --- |
| docling | yes | 2.107.0 | MIT |
| markitdown | yes | 0.1.6 | MIT |
| oxide | yes | oxide 0.1.0 | MIT OR Apache-2.0 |
| pdf_oxide | yes | 0.3.68 | MIT |
| pdfminer.six | yes | 20251230 | MIT |
| pdfplumber | yes | 0.11.9 | MIT |
| pdftext | yes | 0.6.3 | Apache-2.0 |
| pdftoppm | available, not run | pdftoppm version 26.02.0 | GPL-2.0-or-later |
| poppler | yes | pdftotext version 26.02.0 | GPL-2.0-or-later |
| pymupdf | yes | 1.27.2.3 | AGPL-3.0/commercial |
| pymupdf4llm | yes | 1.27.2.3 | AGPL-3.0/commercial |
| pypdf | yes | 6.14.2 | BSD-3-Clause |
| pypdfium2 | yes | 4.30.0 | Apache-2.0/BSD-3-Clause |
| qpdf | available, not run | qpdf version 12.3.2 | Apache-2.0 |

## Speed And Pass Rate: Text Extraction

| ranked tool | pass % | mean s | p50 s | p95 s | p99 s | mem p95 MB | docs/sec |
| --- | --- | --- | --- | --- | --- | --- | --- |
| oxide | 100.000 | 0.118 | 0.108 | 0.191 | 0.294 | 1.996 | 8.4613 |
| poppler | 100.000 | 0.159 | 0.109 | 0.456 | 0.691 | 7.942 | 6.2943 |
| pymupdf | 100.000 | 0.383 | 0.310 | 0.983 | 1.222 | 54.157 | 2.6128 |
| pdfminer.six | 100.000 | 0.422 | 0.311 | 1.080 | 1.468 | 43.028 | 2.3702 |
| pdf_oxide | 100.000 | 0.431 | 0.311 | 1.139 | 1.529 | 119.175 | 2.3223 |
| pdfplumber | 100.000 | 0.445 | 0.311 | 1.073 | 1.433 | 39.430 | 2.2449 |
| pypdfium2 | 100.000 | 0.480 | 0.318 | 0.993 | 1.964 | 35.755 | 2.0852 |
| pypdf | 100.000 | 0.496 | 0.312 | 1.218 | 1.852 | 49.983 | 2.0142 |
| pdftext | 100.000 | 0.923 | 0.728 | 1.912 | 2.499 | 50.782 | 1.0832 |
| markitdown | 100.000 | 2.292 | 1.865 | 4.330 | 6.090 | 191.332 | 0.4363 |
| pymupdf4llm | 100.000 | 3.317 | 2.494 | 8.243 | 14.768 | 486.672 | 0.3015 |
| docling | 100.000 | 18.090 | 15.255 | 35.343 | 51.572 | 4.891 | 0.0553 |

Files nobody passed for text extraction: 0.

### Per-Category Text Speed/Pass Rate

#### has-images
| tool | attempted | pass % | mean s | p95 s |
| --- | --- | --- | --- | --- |
| oxide | 200 | 100.000 | 0.118 | 0.191 |
| poppler | 200 | 100.000 | 0.159 | 0.456 |
| pymupdf | 200 | 100.000 | 0.383 | 0.983 |
| pdfminer.six | 200 | 100.000 | 0.422 | 1.080 |
| pdf_oxide | 200 | 100.000 | 0.431 | 1.139 |
| pdfplumber | 200 | 100.000 | 0.445 | 1.073 |
| pypdfium2 | 200 | 100.000 | 0.480 | 0.993 |
| pypdf | 200 | 100.000 | 0.496 | 1.218 |
| pdftext | 200 | 100.000 | 0.923 | 1.912 |
| markitdown | 200 | 100.000 | 2.292 | 4.330 |
| pymupdf4llm | 200 | 100.000 | 3.317 | 8.243 |
| docling | 200 | 100.000 | 18.090 | 35.343 |

#### image-heavy
| tool | attempted | pass % | mean s | p95 s |
| --- | --- | --- | --- | --- |
| oxide | 200 | 100.000 | 0.118 | 0.191 |
| poppler | 200 | 100.000 | 0.159 | 0.456 |
| pymupdf | 200 | 100.000 | 0.383 | 0.983 |
| pdfminer.six | 200 | 100.000 | 0.422 | 1.080 |
| pdf_oxide | 200 | 100.000 | 0.431 | 1.139 |
| pdfplumber | 200 | 100.000 | 0.445 | 1.073 |
| pypdfium2 | 200 | 100.000 | 0.480 | 0.993 |
| pypdf | 200 | 100.000 | 0.496 | 1.218 |
| pdftext | 200 | 100.000 | 0.923 | 1.912 |
| markitdown | 200 | 100.000 | 2.292 | 4.330 |
| pymupdf4llm | 200 | 100.000 | 3.317 | 8.243 |
| docling | 200 | 100.000 | 18.090 | 35.343 |

#### image-heavy-count
| tool | attempted | pass % | mean s | p95 s |
| --- | --- | --- | --- | --- |
| oxide | 70 | 100.000 | 0.117 | 0.123 |
| poppler | 70 | 100.000 | 0.170 | 0.469 |
| pymupdf | 70 | 100.000 | 0.425 | 0.968 |
| pdfminer.six | 70 | 100.000 | 0.425 | 0.954 |
| pdfplumber | 70 | 100.000 | 0.430 | 0.850 |
| pypdfium2 | 70 | 100.000 | 0.479 | 1.021 |
| pypdf | 70 | 100.000 | 0.505 | 1.048 |
| pdf_oxide | 70 | 100.000 | 0.526 | 1.240 |
| pdftext | 70 | 100.000 | 0.951 | 1.979 |
| markitdown | 70 | 100.000 | 2.398 | 4.203 |
| pymupdf4llm | 70 | 100.000 | 5.325 | 12.758 |
| docling | 70 | 100.000 | 24.310 | 49.303 |

#### large-file
| tool | attempted | pass % | mean s | p95 s |
| --- | --- | --- | --- | --- |
| oxide | 65 | 100.000 | 0.118 | 0.124 |
| poppler | 65 | 100.000 | 0.174 | 0.456 |
| pdfminer.six | 65 | 100.000 | 0.431 | 0.985 |
| pdfplumber | 65 | 100.000 | 0.432 | 0.857 |
| pymupdf | 65 | 100.000 | 0.444 | 1.009 |
| pypdfium2 | 65 | 100.000 | 0.486 | 1.034 |
| pypdf | 65 | 100.000 | 0.505 | 1.046 |
| pdf_oxide | 65 | 100.000 | 0.529 | 1.254 |
| pdftext | 65 | 100.000 | 0.914 | 1.764 |
| markitdown | 65 | 100.000 | 2.235 | 4.136 |
| pymupdf4llm | 65 | 100.000 | 5.525 | 13.373 |
| docling | 65 | 100.000 | 24.821 | 50.046 |

#### medium-doc
| tool | attempted | pass % | mean s | p95 s |
| --- | --- | --- | --- | --- |
| oxide | 44 | 100.000 | 0.125 | 0.202 |
| poppler | 44 | 100.000 | 0.165 | 0.485 |
| pdfplumber | 44 | 100.000 | 0.426 | 0.882 |
| pdfminer.six | 44 | 100.000 | 0.436 | 0.883 |
| pymupdf | 44 | 100.000 | 0.458 | 1.063 |
| pypdfium2 | 44 | 100.000 | 0.489 | 1.036 |
| pypdf | 44 | 100.000 | 0.526 | 1.281 |
| pdf_oxide | 44 | 100.000 | 0.591 | 1.424 |
| pdftext | 44 | 100.000 | 0.850 | 1.773 |
| markitdown | 44 | 100.000 | 2.181 | 3.873 |
| pymupdf4llm | 44 | 100.000 | 6.256 | 14.627 |
| docling | 44 | 100.000 | 28.620 | 51.408 |

#### short-doc
| tool | attempted | pass % | mean s | p95 s |
| --- | --- | --- | --- | --- |
| oxide | 156 | 100.000 | 0.116 | 0.189 |
| poppler | 156 | 100.000 | 0.157 | 0.425 |
| pymupdf | 156 | 100.000 | 0.361 | 0.873 |
| pdf_oxide | 156 | 100.000 | 0.385 | 0.929 |
| pdfminer.six | 156 | 100.000 | 0.418 | 1.145 |
| pdfplumber | 156 | 100.000 | 0.451 | 1.150 |
| pypdfium2 | 156 | 100.000 | 0.477 | 0.893 |
| pypdf | 156 | 100.000 | 0.488 | 1.160 |
| pdftext | 156 | 100.000 | 0.944 | 1.933 |
| markitdown | 156 | 100.000 | 2.323 | 4.512 |
| pymupdf4llm | 156 | 100.000 | 2.487 | 4.596 |
| docling | 156 | 100.000 | 15.120 | 19.370 |

## Accuracy Against Ground Truth

Text scoring normalizes whitespace, then reports character similarity, token F1, ground-truth line recall, spurious line ratio, and order correctness from matched ground-truth line positions. It penalizes missing lines and extra text.

| tool | scored | char sim | word F1 | line recall | spurious ratio | order |
| --- | --- | --- | --- | --- | --- | --- |
| oxide | 200 | 0.927 | 1.000 | 1.000 | 0.076 | 0.960 |
| pdf_oxide | 200 | 0.925 | 1.000 | 1.000 | 0.078 | 0.960 |
| pdfminer.six | 200 | 0.906 | 1.000 | 1.000 | 0.000 | 0.956 |
| pdfplumber | 200 | 0.937 | 1.000 | 1.000 | 0.076 | 0.960 |
| pypdf | 200 | 0.996 | 1.000 | 1.000 | 0.000 | 0.999 |
| pdftext | 200 | 0.994 | 1.000 | 1.000 | 0.075 | 0.999 |
| poppler | 200 | 0.915 | 0.986 | 0.946 | 0.076 | 0.915 |
| pymupdf | 200 | 0.988 | 0.986 | 0.939 | 0.000 | 0.944 |
| pypdfium2 | 200 | 0.973 | 0.986 | 0.939 | 0.075 | 0.944 |
| markitdown | 200 | 0.901 | 0.966 | 1.000 | 0.032 | 0.956 |
| docling | 200 | 0.405 | 0.285 | 0.463 | 0.784 | 0.670 |
| pymupdf4llm | 200 | 0.340 | 0.277 | 0.940 | 0.637 | 0.889 |

### Table Accuracy

Table scoring compares ground-truth headers+cells to structured table outputs. False table detections count against precision.

This capped slice contains no table ground truth, so table metrics are not scored here.

| tool | scored | cell F1 | recall | precision | TEDS approx |
| --- | --- | --- | --- | --- | --- |
| oxide | 0 | - | - | - | - |
| pymupdf | 0 | - | - | - | - |
| pdfplumber | 0 | - | - | - | - |

Tools not shown lack structured table extraction in this harness or were not installed.

### Field / Key-Value Accuracy

This capped slice contains no field ground truth, so field metrics are not scored here.

| tool | scored | strict field F1 | recall | precision | value-only F1 |
| --- | --- | --- | --- | --- | --- |
| oxide | 0 | - | - | - | - |
| pypdf | 0 | - | - | - | - |

Strict field F1 requires key and value to match; value-only F1 shows values found under different labels.

### Image Count Accuracy

| tool | scored | count accuracy | mean abs error |
| --- | --- | --- | --- |
| oxide | 200 | 1.000 | 0.000 |
| pymupdf | 200 | 1.000 | 0.000 |
| pdfplumber | 200 | 1.000 | 0.000 |
| pypdf | 200 | 1.000 | 0.000 |
| poppler | 188 | 1.000 | 0.000 |

## Capability Matrix

| capability | oxide | pdf_oxide | pymupdf | pypdfium2 | pymupdf4llm | pdftext | pdfminer.six | pdfplumber | markitdown | pypdf | docling | qpdf | poppler |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| plain text extraction | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | no | yes |
| chars/words/lines with geometry | partial | yes | yes | partial | partial | yes | partial | yes | no | partial | yes | no | partial |
| layout/reading-order structure | yes | yes | partial | no | yes | yes | no | partial | no | no | yes | no | no |
| table extraction | yes | yes | yes | partial | yes | partial | no | yes | partial | no | yes | no | no |
| image extraction/counting | yes | yes | yes | partial | no | no | partial | partial | no | partial | no | no | yes |
| form field read/fill | partial | yes | yes | partial | no | no | no | no | no | partial | partial | partial | partial |
| markdown conversion | partial | yes | no | no | yes | no | no | no | yes | no | yes | no | no |
| region/scoped extraction | yes | yes | yes | partial | partial | partial | partial | yes | no | partial | partial | no | no |
| extraction profiles | yes | yes | no | no | partial | no | no | no | no | no | partial | no | no |
| Python/developer API | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | no | no |
| OCR | partial | yes | partial | no | partial | no | no | no | partial | no | yes | no | partial |
| repair/linearization/validation | yes | no | partial | no | no | no | no | no | no | partial | no | yes | partial |
| digital signatures/PDF-A/PDF-UA | yes | no | partial | no | no | no | no | no | no | partial | no | no | no |
| MCP/AI assistant integration | no | yes | no | no | no | no | no | no | partial | no | partial | no | no |

### What Oxide Lacks

- Broad published language binding set comparable to pdf_oxide's Python/Go/JS/C#/.NET/Java/WASM positioning.
- Built-in OCR runtime; Oxide OCR is optional and Tesseract-backed rather than bundled.
- Docling-class ML layout/OCR is not built into Oxide; this release binary is not OCR-enabled.
- Documented XFA form data support comparable to pdf_oxide's advertised XFA capability.
- MCP server / AI assistant integration.
- MCP server/assistant integration advertised by pdf_oxide.
- qpdf remains the stronger dedicated structural validator and repair reference.

### What Oxide Uniquely Has / Where It Is Strong

- C ABI and WASM bindings in-tree in addition to the Rust library and CLI.
- Digital signature signing, verification, coverage, trust, timestamp/LTV reporting, and offline DSS material support.
- Enterprise-oriented operations beyond extraction: render, split, merge, rotate, optimize, repair, linearize, redact, attachments, server deployment.
- PDF/A and PDF/UA validation/conversion surfaces in the Rust API/CLI.
- PDF/A/PDF/UA and digital-signature surfaces exceed most extraction-only tools.
- Pure-Rust core with CLI, Rust library, Python binding, C ABI, and WASM surfaces.
- Pure-Rust encryption/decryption support including RC4 legacy, AES-128, and AES-256 paths, with documented crypto review.
- Region extraction, extraction profiles, and markdown heading detection are exposed across Rust, CLI, and Python.
- Security posture documentation, sanitizer CI, fuzzing, dependency policy, and hostile-corpus safety gates.
- Self-host HTTP API with auth, rate limits, resource caps, and async render/image jobs.
- Self-host HTTP server with async job API, auth, rate limiting, file/time/output caps, and JSON endpoints.
- Single product surface spans parse, tables, fields, images, render, edit, optimize, repair, linearize, redact, encrypt, and signatures.

Capability source notes: docling: https://docling-project.github.io/docling/; markitdown: https://github.com/microsoft/markitdown, https://pypi.org/project/markitdown/; oxide: docs/api_overview.md, docs/self_hosting.md, docs/signatures.md, docs/tables.md, docs/semantic_extraction.md; pdf_oxide: https://docs.rs/pdf_oxide/latest/pdf_oxide/, https://pdf.oxide.fyi/; pdfminer.six: https://pdfminersix.readthedocs.io/; pdfplumber: https://github.com/jsvine/pdfplumber; pdftext: https://pypi.org/project/pdftext/, https://github.com/datalab-to/pdftext; poppler: https://poppler.freedesktop.org/; pymupdf: https://pymupdf.readthedocs.io/, https://pymupdf.io/; pymupdf4llm: https://pymupdf.readthedocs.io/en/latest/pymupdf4llm/, https://pymupdf.readthedocs.io/en/latest/pymupdf4llm/api.html; pypdf: https://pypdf.readthedocs.io/, https://github.com/py-pdf/pypdf; pypdfium2: https://pypdfium2.readthedocs.io/, https://github.com/pypdfium2-team/pypdfium2; qpdf: https://qpdf.readthedocs.io/

## Blunt Verdict

Oxide is fastest by mean text wall time among tools that ran: 0.118s mean, ahead of Poppler at 0.159s.
Oxide does not lead text character fidelity. Oxide char-sim is 0.927; higher char-sim tools are pdfplumber, pymupdf, pypdfium2, pypdf, and pdftext.
Oxide leads or ties text word-F1 among tools that ran.
Docling was measured through Python 3.12 and passed all 200 PDFs for text, but it was much slower here and scored poorly on this synthetic text metric.

## Before / After Snapshot

| metric | campaign baseline | 200-PDF capped result | status |
| --- | --- | --- | --- |
| text char-sim | 0.742 | 0.927 | improved, still trails the best char-sim tools |
| text word-F1 | 0.886 | 1.000 | improved |
| image count accuracy | not in baseline summary | 1.000 | matched available competitors on scored files |
| table precision / TEDS | 0.806 / 0.667 | not scored in this capped slice | use the dedicated table slice for table claims |
| strict field-F1 | 0.104 | not scored in this capped slice | use the dedicated field slice for field claims |

## State Of The Project

Oxide has a proper CLI and developer API: the CLI has structured output and classified errors, and the Python binding exposes the expected document/page ergonomics. The code/docs side of enterprise hardening is in place: MSRV is pinned, the RSA advisory decision is documented, and the stability/security docs are current. This is not a claim of certification or external audit.

## Prioritized Fix List

1. **Character fidelity gap**: close the remaining char-sim gap versus pypdf/pdftext/PyMuPDF.
2. **Table and field scorecard coverage**: keep table and field claims tied to their dedicated 200-PDF slices, not this image-heavy slice.
3. **Enterprise proof beyond code**: external audit and longer-running wild-PDF exposure remain outside this code-only campaign.

