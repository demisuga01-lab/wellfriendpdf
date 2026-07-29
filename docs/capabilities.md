# Wellfriend PDF SDK capabilities

Wellfriend is a source-linked PDF SDK for inspection, rendering, extraction, true editing, document repair, redaction, sanitization, signing workflows, forms, annotations, table/math/OCR subsystems, and language bindings over one canonical Rust core.

Capability labels in the README use this scale:

- **Verified**: exercised by repository tests, benchmark smoke, or validation artifacts on supported fixtures.
- **Verified with limits**: implemented and tested for the documented boundary, with typed refusal outside that boundary.
- **Supported**: implemented API exists; deeper corpus validation is covered by engineering validation reports.
- **Review required**: the engine can plan or reconstruct, but a low-confidence or destructive decision requires caller approval.
- **Typed refusal outside scope**: unsupported or ambiguous input returns a specific error without modifying the document.

## Core document operations

| Area | Current support |
|---|---|
| Parsing and structure | COS graph, xref repair paths, object streams, encryption reporting, page tree operations, deterministic serialization. |
| Rendering and extraction | In-process page rendering, text extraction, structured document model, tables and fields extraction, image extraction, HTML/Office exports. |
| True editing | Operator-preserving source edits, scene/transaction edits, text reflow, tables/math/OCR/form/annotation edits, undo reports. |
| Security | Source redaction, residual verification, sanitization, risky-content scan, signature impact reporting. |
| Standards | Internal PDF/A, PDF/UA, PDF/X, WTPDF-oriented checks plus recorded external-tool availability boundaries. |
| Bindings | Rust, CLI, Python, C ABI, WASM, .NET, Java/Maven, Java/Gradle, and server crate surfaces. |

## Editing modes

| Mode | Purpose | Boundary |
|---|---|---|
| Operator-preserving | Patch a resolved text/path/source operator while preserving the surrounding file structure. | Same-width/source-safe operator edits; ambiguity refuses. |
| Geometric block | Reflow text inside an explicit page/region geometry and move only source-linked downstream objects. | Requires bounded region, known neighbors, and reversible transaction. |
| Semantic document | Reconstruct document flow for columns, pages, lists, headings, captions, footnotes, tables, and forms. | Low confidence or destructive inference requires review. |

## Common typed limits

Unsupported dynamic XFA conversion, ambiguous semantic flow, unsafe glyph reconstruction, unresolved source text, signature-policy conflicts, low-confidence OCR, unavailable external providers, and destructive actions without approval all fail closed with typed results.
