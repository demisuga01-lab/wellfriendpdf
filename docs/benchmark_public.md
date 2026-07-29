# Public PDF Text Extraction Benchmark

Generated: 2026-06-24T00:59:37.570075+00:00
Commit: `eaf5f49d9fb3f4a27a7b495d98955bc8a43019c0`

## Scope And Method

- Corpus files in manifest: 20
- Files benchmarked in this run: 20
- Timeout per tool/file: 20s
- Method: single-thread env, no warm-up, one isolated subprocess per tool/file.
- Pass definition: subprocess exits 0 before timeout/memory cap and writes text output.
- Nobody-passed files: 0

Raw PDFs and per-file raw results are local-only and ignored by git. The manifest records source URLs and hashes for reproducibility.

## Corpus Breakdown

| tag/category      | files |
| ----------------- | ----- |
| cjk-text          | 4     |
| digital-born      | 13    |
| multilang         | 5     |
| pathological      | 3     |
| pdfa              | 6     |
| pdfa-pdfua        | 6     |
| pdfjs-fixtures    | 7     |
| safedocs-targeted | 3     |

## Tool Availability

| tool         | available | reason/license                                                 |
| ------------ | --------- | -------------------------------------------------------------- |
| wellfriendpdf        | yes       | MIT                                              |
| pdf_wellfriendpdf    | yes       | MIT                                                            |
| pymupdf      | yes       | AGPL-3.0/commercial                                            |
| pypdfium2    | yes       | Apache-2.0/BSD-3-Clause                                        |
| pymupdf4llm  | yes       | AGPL-3.0/commercial                                            |
| pdftext      | yes       | Apache-2.0                                                     |
| pdfminer.six | yes       | MIT                                                            |
| pdfplumber   | yes       | MIT                                                            |
| markitdown   | yes       | MIT                                                            |
| pypdf        | yes       | BSD-3-Clause                                                   |
| oxidize_pdf  | no        | optional Rust text harness command not found: oxidize_pdf_text |
| unpdf        | no        | optional Rust text harness command not found: unpdf_text       |
| pdf_extract  | no        | optional Rust text harness command not found: pdf_extract_text |
| lopdf        | no        | optional Rust text harness command not found: lopdf_text       |

## Overall Head-To-Head

| tool         | pass %  | mean s | p50 s | p95 s | p99 s | mem p95 MB |
| ------------ | ------- | ------ | ----- | ----- | ----- | ---------- |
| wellfriendpdf        | 95.000  | 0.063  | 0.034 | 0.097 | 0.379 | 5.471      |
| pdf_wellfriendpdf    | 100.000 | 0.167  | 0.161 | 0.186 | 0.188 | 3.882      |
| pymupdf      | 95.000  | 0.187  | 0.185 | 0.210 | 0.212 | 3.947      |
| pypdfium2    | 95.000  | 0.286  | 0.287 | 0.342 | 0.362 | 3.980      |
| pymupdf4llm  | 95.000  | 1.068  | 1.039 | 1.293 | 1.421 | 3.921      |
| pdftext      | 95.000  | 0.466  | 0.446 | 0.530 | 0.602 | 4.097      |
| pdfminer.six | 95.000  | 0.263  | 0.262 | 0.288 | 0.289 | 4.329      |
| pdfplumber   | 95.000  | 0.280  | 0.287 | 0.295 | 0.311 | 3.883      |
| markitdown   | 95.000  | 0.761  | 0.749 | 0.828 | 0.846 | 3.924      |
| pypdf        | 95.000  | 0.240  | 0.238 | 0.261 | 0.263 | 4.026      |

## Per-Category Results

### cjk-text

| tool         | files | pass %  | mean s | p95 s |
| ------------ | ----- | ------- | ------ | ----- |
| wellfriendpdf        | 4     | 100.000 | 0.037  | 0.051 |
| pdf_wellfriendpdf    | 4     | 100.000 | 0.167  | 0.182 |
| pymupdf      | 4     | 100.000 | 0.185  | 0.186 |
| pypdfium2    | 4     | 100.000 | 0.321  | 0.363 |
| pymupdf4llm  | 4     | 100.000 | 1.115  | 1.393 |
| pdftext      | 4     | 100.000 | 0.482  | 0.516 |
| pdfminer.six | 4     | 100.000 | 0.268  | 0.286 |
| pdfplumber   | 4     | 100.000 | 0.287  | 0.311 |
| markitdown   | 4     | 100.000 | 0.762  | 0.793 |
| pypdf        | 4     | 100.000 | 0.237  | 0.238 |

### digital-born

| tool         | files | pass %  | mean s | p95 s |
| ------------ | ----- | ------- | ------ | ----- |
| wellfriendpdf        | 13    | 100.000 | 0.073  | 0.214 |
| pdf_wellfriendpdf    | 13    | 100.000 | 0.168  | 0.187 |
| pymupdf      | 13    | 100.000 | 0.187  | 0.211 |
| pypdfium2    | 13    | 100.000 | 0.275  | 0.300 |
| pymupdf4llm  | 13    | 100.000 | 1.037  | 1.213 |
| pdftext      | 13    | 100.000 | 0.464  | 0.531 |
| pdfminer.six | 13    | 100.000 | 0.264  | 0.288 |
| pdfplumber   | 13    | 100.000 | 0.278  | 0.289 |
| markitdown   | 13    | 100.000 | 0.761  | 0.835 |
| pypdf        | 13    | 100.000 | 0.241  | 0.262 |

### multilang

| tool         | files | pass %  | mean s | p95 s |
| ------------ | ----- | ------- | ------ | ----- |
| wellfriendpdf        | 5     | 80.000  | 0.037  | 0.051 |
| pdf_wellfriendpdf    | 5     | 100.000 | 0.166  | 0.180 |
| pymupdf      | 5     | 80.000  | 0.185  | 0.186 |
| pypdfium2    | 5     | 80.000  | 0.321  | 0.363 |
| pymupdf4llm  | 5     | 80.000  | 1.115  | 1.393 |
| pdftext      | 5     | 80.000  | 0.482  | 0.516 |
| pdfminer.six | 5     | 80.000  | 0.268  | 0.286 |
| pdfplumber   | 5     | 80.000  | 0.287  | 0.311 |
| markitdown   | 5     | 80.000  | 0.762  | 0.793 |
| pypdf        | 5     | 80.000  | 0.237  | 0.238 |

### pathological

| tool         | files | pass %  | mean s | p95 s |
| ------------ | ----- | ------- | ------ | ----- |
| wellfriendpdf        | 3     | 66.667  | 0.045  | 0.057 |
| pdf_wellfriendpdf    | 3     | 100.000 | 0.162  | 0.164 |
| pymupdf      | 3     | 66.667  | 0.185  | 0.187 |
| pypdfium2    | 3     | 66.667  | 0.286  | 0.310 |
| pymupdf4llm  | 3     | 66.667  | 1.171  | 1.205 |
| pdftext      | 3     | 66.667  | 0.443  | 0.466 |
| pdfminer.six | 3     | 66.667  | 0.248  | 0.261 |
| pdfplumber   | 3     | 66.667  | 0.278  | 0.291 |
| markitdown   | 3     | 66.667  | 0.762  | 0.798 |
| pypdf        | 3     | 66.667  | 0.237  | 0.238 |

### pdfa

| tool         | files | pass %  | mean s | p95 s |
| ------------ | ----- | ------- | ------ | ----- |
| wellfriendpdf        | 6     | 100.000 | 0.045  | 0.058 |
| pdf_wellfriendpdf    | 6     | 100.000 | 0.168  | 0.188 |
| pymupdf      | 6     | 100.000 | 0.186  | 0.204 |
| pypdfium2    | 6     | 100.000 | 0.276  | 0.308 |
| pymupdf4llm  | 6     | 100.000 | 1.016  | 1.153 |
| pdftext      | 6     | 100.000 | 0.451  | 0.471 |
| pdfminer.six | 6     | 100.000 | 0.263  | 0.288 |
| pdfplumber   | 6     | 100.000 | 0.275  | 0.289 |
| markitdown   | 6     | 100.000 | 0.754  | 0.832 |
| pypdf        | 6     | 100.000 | 0.238  | 0.241 |

### pdfa-pdfua

| tool         | files | pass %  | mean s | p95 s |
| ------------ | ----- | ------- | ------ | ----- |
| wellfriendpdf        | 6     | 100.000 | 0.045  | 0.058 |
| pdf_wellfriendpdf    | 6     | 100.000 | 0.168  | 0.188 |
| pymupdf      | 6     | 100.000 | 0.186  | 0.204 |
| pypdfium2    | 6     | 100.000 | 0.276  | 0.308 |
| pymupdf4llm  | 6     | 100.000 | 1.016  | 1.153 |
| pdftext      | 6     | 100.000 | 0.451  | 0.471 |
| pdfminer.six | 6     | 100.000 | 0.263  | 0.288 |
| pdfplumber   | 6     | 100.000 | 0.275  | 0.289 |
| markitdown   | 6     | 100.000 | 0.754  | 0.832 |
| pypdf        | 6     | 100.000 | 0.238  | 0.241 |

### pdfjs-fixtures

| tool         | files | pass %  | mean s | p95 s |
| ------------ | ----- | ------- | ------ | ----- |
| wellfriendpdf        | 7     | 100.000 | 0.098  | 0.331 |
| pdf_wellfriendpdf    | 7     | 100.000 | 0.167  | 0.185 |
| pymupdf      | 7     | 100.000 | 0.189  | 0.205 |
| pypdfium2    | 7     | 100.000 | 0.274  | 0.290 |
| pymupdf4llm  | 7     | 100.000 | 1.055  | 1.227 |
| pdftext      | 7     | 100.000 | 0.476  | 0.575 |
| pdfminer.six | 7     | 100.000 | 0.266  | 0.281 |
| pdfplumber   | 7     | 100.000 | 0.281  | 0.289 |
| markitdown   | 7     | 100.000 | 0.767  | 0.825 |
| pypdf        | 7     | 100.000 | 0.244  | 0.262 |

### safedocs-targeted

| tool         | files | pass %  | mean s | p95 s |
| ------------ | ----- | ------- | ------ | ----- |
| wellfriendpdf        | 3     | 66.667  | 0.045  | 0.057 |
| pdf_wellfriendpdf    | 3     | 100.000 | 0.162  | 0.164 |
| pymupdf      | 3     | 66.667  | 0.185  | 0.187 |
| pypdfium2    | 3     | 66.667  | 0.286  | 0.310 |
| pymupdf4llm  | 3     | 66.667  | 1.171  | 1.205 |
| pdftext      | 3     | 66.667  | 0.443  | 0.466 |
| pdfminer.six | 3     | 66.667  | 0.248  | 0.261 |
| pdfplumber   | 3     | 66.667  | 0.278  | 0.291 |
| markitdown   | 3     | 66.667  | 0.762  | 0.798 |
| pypdf        | 3     | 66.667  | 0.237  | 0.238 |

## Text Quality Sample

- Sampled files: 16
- Reference tool selection: {'pymupdf': 16}
| tool         | files | mean word ratio | mean char ratio |
| ------------ | ----- | --------------- | --------------- |
| wellfriendpdf        | 16    | 0.656           | 0.741           |
| pdf_wellfriendpdf    | 16    | 0.750           | 0.710           |
| pypdfium2    | 16    | 0.840           | 0.801           |
| pymupdf4llm  | 16    | 0.788           | 0.710           |
| pdftext      | 16    | 0.808           | 0.795           |
| pdfminer.six | 16    | 0.738           | 0.739           |
| pdfplumber   | 16    | 0.738           | 0.737           |
| markitdown   | 16    | 0.738           | 0.739           |
| pypdf        | 16    | 0.733           | 0.769           |

## Capability Matrix

| capability                                   | wellfriendpdf   | pdf_wellfriendpdf | pymupdf | pypdfium2 | pymupdf4llm | pdftext | pdfminer.six | pdfplumber | markitdown | pypdf   |
| -------------------------------------------- | ------- | --------- | ------- | --------- | ----------- | ------- | ------------ | ---------- | ---------- | ------- |
| plain text extraction                        | yes     | yes       | yes     | yes       | yes         | yes     | yes          | yes        | yes        | yes     |
| chars/words/lines with geometry              | partial | yes       | yes     | partial   | partial     | yes     | partial      | yes        | no         | partial |
| layout-aware reading order / document model  | yes     | yes       | partial | partial   | yes         | yes     | partial      | partial    | partial    | partial |
| table extraction                             | yes     | yes       | yes     | partial   | yes         | partial | no           | yes        | partial    | no      |
| image extraction                             | yes     | yes       | yes     | partial   | yes         | no      | partial      | partial    | partial    | partial |
| form field read/fill                         | partial | yes       | yes     | partial   | no          | no      | no           | no         | no         | partial |
| annotations                                  | partial | yes       | yes     | partial   | no          | no      | partial      | partial    | no         | yes     |
| markdown conversion with heading detection   | partial | yes       | partial | no        | yes         | no      | no           | no         | yes        | no      |
| HTML conversion                              | yes     | yes       | yes     | no        | no          | no      | yes          | no         | no         | no      |
| PDF creation                                 | yes     | yes       | yes     | yes       | no          | no      | no           | no         | no         | yes     |
| editing / merge / split / rotate / redaction | yes     | yes       | yes     | partial   | no          | no      | no           | no         | no         | yes     |
| rendering                                    | yes     | yes       | yes     | yes       | partial     | no      | no           | partial    | no         | no      |
| OCR                                          | partial | yes       | partial | no        | partial     | no      | no           | no         | partial    | no      |
| encryption / password handling               | yes     | yes       | yes     | partial   | partial     | partial | yes          | partial    | partial    | yes     |
| digital signatures / LTV reporting           | yes     | unknown   | partial | no        | no          | no      | no           | no         | no         | partial |
| PDF/A or PDF/UA validation/conversion        | yes     | unknown   | partial | no        | no          | no      | no           | no         | no         | no      |
| metadata                                     | yes     | yes       | yes     | yes       | yes         | partial | yes          | yes        | partial    | yes     |
| search                                       | partial | yes       | yes     | partial   | no          | no      | no           | yes        | no         | no      |
| region/scoped extraction                     | no      | yes       | yes     | partial   | partial     | partial | partial      | yes        | no         | partial |
| extraction profiles / preset strategies      | partial | yes       | partial | no        | yes         | partial | partial      | partial    | partial    | no      |
| language bindings                            | partial | yes       | partial | python    | python      | python  | python       | python     | python     | python  |
| CLI                                          | yes     | yes       | partial | yes       | partial     | partial | yes          | yes        | yes        | partial |
| self-host HTTP/API server                    | yes     | no        | no      | no        | no          | no      | no           | no         | no         | no      |
| MCP / AI assistant integration               | no      | yes       | no      | no        | partial     | no      | no           | no         | partial    | no      |

## Feature Gaps Found

- Region/scoped extraction API comparable to pdf_wellfriendpdf page.region()/within() and pdfplumber crop().
- Documented extraction-profile presets comparable to pdf_wellfriendpdf ExtractionProfile and PyMuPDF4LLM's high-level extraction modes.
- Lazy Python page properties such as page.text/page.words/page.tables/page.images.
- Broad published language binding set comparable to pdf_wellfriendpdf's Python/Go/JS/C#/.NET/Java/WASM positioning.
- MCP server / AI assistant integration.
- Built-in OCR runtime; Wellfriend OCR is optional and Tesseract-backed rather than bundled.
- Documented XFA form data support comparable to pdf_wellfriendpdf's advertised XFA capability.
- Markdown heading detection is present only as heuristic document-model output, not an explicit page.markdown(detect_headings=True)-style API.

## Wellfriend Differentiators Found

- Self-host HTTP server with async job API, auth, rate limiting, file/time/output caps, and JSON endpoints.
- PDF/A and PDF/UA validation/conversion surfaces in the Rust API/CLI.
- Digital signature signing, verification, coverage, trust, timestamp/LTV reporting, and offline DSS material support.
- Pure-Rust encryption/decryption support including RC4 legacy, AES-128, and AES-256 paths, with documented crypto review.
- C ABI and WASM bindings in-tree in addition to the Rust library and CLI.
- Enterprise-oriented operations beyond extraction: render, split, merge, rotate, optimize, repair, linearize, redact, attachments, server deployment.
- Security posture documentation, sanitizer CI, fuzzing, dependency policy, and hostile-corpus safety gates.

## Prioritized Work List

1. **fidelity**: Wellfriend failed files that another extractor passed. Evidence: `["darpa-safedocs_Unicode_passwords_corrigendum4_unicode-test-U2F874-correct"]`
2. **text-quality**: Some tools diverged materially from the reference text sample; inspect Wellfriend if listed. Evidence: `{"wellfriendpdf": {"mean_word_ratio": 0.6557, "mean_char_ratio": 0.74114, "files": 16}, "pdf_wellfriendpdf": {"mean_word_ratio": 0.74966, "mean_char_ratio": 0.70997, "files": 16}, "pypdfium2": {"mean_word_ratio": 0.83969, "mean_char_ratio": 0.80096, "files": 16}, "pymupdf4llm": {"mean_word_ratio": 0.78765, "mean_char_ratio": 0.7104, "files": 16}, "pdftext": {"mean_word_ratio": 0.80844, "mean_char_ratio": 0.79488, "files": 16}, "pdfminer.six": {"mean_word_ratio": 0.73808, "mean_char_ratio": 0.73916, "files": 16}, "pdfplumber": {"mean_word_ratio": 0.73792, "mean_char_ratio": 0.73656, "files": 16}, "markitdown": {"mean_word_ratio": 0.73808, "mean_char_ratio": 0.73916, "files": 16}, "pypdf": {"mean_word_ratio": 0.73346, "mean_char_ratio": 0.76905, "files": 16}}`

## Provenance

- Python: `3.14.3`
- Platform: `win32`
- Wellfriend binary: `target\release\wellfriendpdf.exe`
- Manifest: `public-benchmark\manifests\public_corpus_manifest.json`
- Output JSON: `public-benchmark\results\raw\smoke-20\results.json`

## Source Notes

- pdf_wellfriendpdf publishes a comparable 3,830-PDF benchmark using veraPDF, Mozilla pdf.js, and DARPA SafeDocs with single-thread, 60s timeout, no warm-up methodology.
- The corpus script also uses arXiv for scale and diversity. arXiv paper license metadata varies by paper; PDFs remain local-only.
