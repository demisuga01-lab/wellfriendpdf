# Office to PDF Architecture

This is the Phase 4e decision record for converting DOCX, PPTX, and XLSX into
PDF. It is authoritative for the default implementation and for any future
external-renderer integration.

## Decision

Oxide ships a native, pure-Rust default path for all three Office directions:

- XLSX to PDF: native grid layout into Oxide's PDF authoring/writer machinery.
- PPTX to PDF: native slide-to-page layout into Oxide's PDF writer.
- DOCX to PDF: native flowing-document layout with the same honest ceiling as
  PDF-to-DOCX: strong structural conversion, not full Word-compatible layout
  reproduction.

LibreOffice/headless `soffice` remains a valid optional future backend, but it
is not a default runtime dependency. If added, it must be feature-gated and use
the same subprocess hygiene as OCR backends: explicit availability checks,
request-level timeout, kill-and-reap on timeout, captured diagnostics, and clear
not-enabled/not-installed messaging.

## Why Not Default to LibreOffice

LibreOffice has a mature command-line conversion path (`soffice --convert-to
pdf ...`) and it is common on Linux self-hosting platforms. The official
LibreOffice help documents `--convert-to` and PDF export filters, so an optional
backend would be practical for deployments that want maximum Office-layout
fidelity and are willing to install LibreOffice.

Oxide's core identity, however, is a pure-Rust PDF engine. Making LibreOffice a
default dependency would change this conversion into orchestration around a
large external runtime. The default path therefore stays native and self-hosted
without hidden external process requirements.

Official references:

- https://help.libreoffice.org/latest/en-US/text/shared/guide/convertfilters.html
- https://help.libreoffice.org/latest/en-US/text/shared/guide/pdf_params.html

## Per-Format Rationale

### XLSX to PDF

XLSX is a grid. Rows, columns, cells, merged ranges, and basic styles can be
mapped directly to PDF drawing operations. The native path parses the OOXML ZIP
parts it needs and writes one or more PDF pages per sheet, splitting wide and
tall sheets into page windows rather than trying to fit an entire sheet onto one
unreadable page.

The native scope preserves cell text, numeric values as displayed text, basic
header styling, and pagination. It does not implement Excel's formula engine or
advanced conditional formatting.

### PPTX to PDF

PPTX slides and PDF pages are both positioned canvases. The native path maps one
slide to one PDF page and places text boxes, tables, and images using the slide
coordinates from DrawingML. This mirrors the Phase 4c PDF-to-PPTX decision in
the opposite direction.

The native scope preserves slide count, text, basic shape position, simple
tables, and embedded PNG/JPEG images. It does not attempt full PowerPoint theme,
animation, SmartArt, chart, or effect rendering.

### DOCX to PDF

DOCX is the hardest reverse direction because it requires paginating flowing
content. The native path parses paragraphs, headings, lists, tables, and images,
then lays them out through Oxide's `FlowDocument` and PDF writer. This gives a
useful self-hosted baseline that preserves document structure, but it is not a
replacement for Word's full layout engine.

Known native limits include exact Word line breaking, section-specific headers
and footers, complex floats, footnotes/endnotes, revision markup, equations, and
theme/font substitution fidelity. A future optional LibreOffice backend can
serve users who need higher layout fidelity and can accept the external runtime.

## Implementation Contract

- Native paths reuse `authoring.rs` and `writer.rs`; no parallel PDF-generation
  stack is allowed.
- Conversion code reads OOXML packages incrementally enough to avoid bitmap or
  page accumulation. The current implementation materializes XML parts and emits
  PDF pages as layout windows are processed; future work can replace individual
  XML scans with streaming pull parsing without changing public APIs.
- Malformed OOXML produces typed Oxide errors, not panics.
- Produced PDFs must open through Oxide's parser and ordinary PDF readers.
- Extraction accuracy slices are not touched by this phase.
