# Competitor and tool landscape

This comparison is purpose-specific. It is not an aggregate ranking and does not treat unavailable tools as losses.

## Directly measured README smoke

| Tool | Version / relationship | README status | Notes |
| --- | --- | --- | --- |
| Wellfriend CLI | current commit `d346915d` | measured_comparable | `extract-text` and `parse --format json` passed on the compact fixture. |
| qpdf | 12.3.2 | measured_comparable | Structural check and linearization passed. qpdf is a structural specialist, not a semantic editor. |
| Poppler | 26.01.0 utilities | measured_comparable | `pdfinfo`, `pdftotext`, and `pdftoppm` passed. |
| pypdfium2 / PDFium | PDFium build `152.0.7947.0` | measured_comparable | Wrapper around PDFium; render smoke passed. |
| PyMuPDF / MuPDF | PyMuPDF 1.28.0, MuPDF 1.29.0 | measured_comparable | MuPDF binding; text/render smoke passed. `mutool` itself was unavailable. |
| pikepdf / qpdf | pikepdf 10.10.0 | measured_comparable | qpdf wrapper; open/save smoke passed. |
| pdfplumber | 0.11.10 | measured_comparable | Extraction smoke passed. Extraction-oriented, not a source-linked editor. |
| Apache PDFBox | 3.0.8 | measured_comparable | Maven smoke opened the compact fixture and reported page count. |
| pyHanko | CLI/runtime available | supported_not_benchmarked | Runtime availability measured; signing/validation capabilities are documentation-oriented here. |

## Exactly unavailable or not measured on the README VPS

| Tool | Status | Reason |
| --- | --- | --- |
| veraPDF | unavailable | Command was not installed on the VPS; no bypass or production change attempted. |
| pdfcpu | unavailable | `go` was unavailable, so the safe `go install` path could not run. |
| OCRmyPDF | unavailable | Command was not installed. |
| Docling | documented_not_benchmarked | Not installed to avoid heavy model/workflow provisioning for a README smoke. |
| PDF.js | documented_not_benchmarked | Browser-focused viewer; no Node/browser benchmark was run. |
| Commercial SDKs | documented_not_benchmarked | No benchmark license was used. |

## Official documentation sources

| Tool | Scope from official source | README evidence class |
| --- | --- | --- |
| PDFium | Chromium PDF library with public embedder APIs. | official_competitor_documentation plus measured wrapper smoke |
| MuPDF | C/Python/JS/.NET document engine for viewing, conversion, manipulation, rendering, extraction, signing, and related workflows. | official_competitor_documentation plus measured PyMuPDF smoke |
| Poppler | PDF rendering library and utilities based on Xpdf. | measured_directly |
| qpdf | Content-preserving structural transformer; not a renderer or text extractor. | measured_directly |
| pikepdf | Pythonic wrapper around qpdf. | measured_directly |
| PDFBox | Java PDF library for creation, manipulation, rendering utilities, and extraction. | measured_directly for open/page count |
| PDF.js | Web standards based PDF parsing/rendering viewer. | documented_not_benchmarked |
| veraPDF | PDF/A and PDF/UA validation specialist. | unavailable on host |
| pyHanko | PDF signature and PAdES tooling. | supported_not_benchmarked |
| pdfcpu | Go API/CLI for validation, optimization, encryption, signing, assembly, extraction, and transformations. | unavailable on host |
| Docling | Document conversion and structured document understanding for AI workflows. | documented_not_benchmarked |
| pdfplumber | pdfminer.six-based text/table/layout extraction for machine-generated PDFs. | measured_directly |
| Camelot | Table extraction from PDFs. | supported_not_benchmarked in README table |
| Tesseract | OCR engine and command-line tool. | validated_in_repository for version availability only |
| OCRmyPDF | Adds OCR text layers to scanned PDFs. | unavailable on host |
| iText | Java/.NET PDF SDK with AGPL/commercial options. | documented_not_benchmarked |
| Apryse | Commercial cross-platform PDF SDK. | documented_not_benchmarked |
| Nutrient | Commercial document engine/SDK platform. | documented_not_benchmarked |
| Foxit PDF SDK | Commercial SDK for rendering, annotations, editing, and cross-platform use. | documented_not_benchmarked |
| Adobe PDF Library / Datalogics | Licensed Adobe PDF engine SDK. | documented_not_benchmarked |

## Rust ecosystem

Rust PDF crates such as `lopdf`, `pdf-writer`, `printpdf`, and `pdf-extract` are relevant ecosystem projects, but the README does not score them as losses. They serve different combinations of low-level manipulation, writing, generation, rendering, or extraction.

## Wellfriend positioning

Wellfriend’s distinctive claim is not that it outrenders every viewer or outvalidates every validator. Its distinctive scope is provenance-linked true editing across source operators, scene/semantic graphs, transactions, undo, reflow, tables, math, OCR layers, forms, annotations, redaction, sanitization, and bindings.
