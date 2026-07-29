# Engineering validation summary

This summary preserves the useful validation evidence without exposing internal roadmap labels as public product language.

## Evidence families

- Parser, xref, object-stream, encryption, and deterministic writer checks.
- Rendering, text extraction, semantic extraction, tables, forms, annotations, and OCR fixture checks.
- Source editing, transaction, undo, provenance, font, shaping, and reflow checks.
- Accessibility repair, redaction residual verification, sanitization, standards checks, signature-impact checks, fuzzing, and sanitizer gates.
- Binding/package checks for Rust, CLI, Python, C ABI, WASM, .NET, Maven, Gradle, and server surfaces.

## How to read the reports

The README only uses direct benchmark numbers and concise user-facing claims. Detailed engineering evidence stays in the docs and evidence folders. A claim is considered README-safe only when it maps to code, tests, benchmark results, or clearly marked documentation-only competitor research.

## Not certification

Repository validation is not a third-party certification. Standards, signatures, accessibility, and commercial interoperability claims remain scoped to the documented validation boundary.
