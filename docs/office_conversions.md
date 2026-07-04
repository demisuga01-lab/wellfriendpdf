# Office Conversions

This page documents the Phase 4 PDF/Office conversions. The shared hierarchy
decision is in `docs/document_hierarchy.md`; the Office-to-PDF architecture
decision is in `docs/office_to_pdf_architecture.md`.

## PDF to XLSX

Entry points:

- Rust: `oxide_engine::pdf_to_xlsx(&engine, &XlsxOptions { ... })`
- CLI: `oxide pdf-to-xlsx input.pdf --out output.xlsx --layout pages`
- Python: `oxide.pdf_to_xlsx("input.pdf", output="output.xlsx", layout="pages")`
- C ABI: `oxide_document_to_xlsx(document, "pages", &out, &error)`

Layout policies:

- `pages` (default): one worksheet per PDF page. Tables and nearby non-table
  text stay in recovered reading order.
- `tables`: one worksheet per detected table, with non-table text placed on a
  `Notes` sheet.

The exporter consumes `BlockKind::Table` from the canonical document hierarchy.
It preserves row/column placement, bolds header cells, maps `rowspan`/`colspan`
to native Excel merged ranges, and writes safely inferable numbers as numeric
cells. Leading-zero identifiers remain text.

Scope boundary: this is a table/data export. Plain prose has no natural Excel
grid, so it is preserved as context rather than forced into fabricated cells.

Validation on this host:

- Digital benchmark files with table ground truth: `invoice.pdf`, `tables.pdf`.
- Independent readback: `openpyxl`.
- Conversion cell precision/recall/F1: `1.0 / 1.0 / 1.0` on that available
  table-bearing digital slice.
- ZIP package integrity: `zipfile.testzip()` returned no bad entries.

## PDF to PPTX

Entry points:

- Rust: `oxide_engine::pdf_to_pptx(&engine, &PptxOptions { ... })`
- CLI: `oxide pdf-to-pptx input.pdf --out output.pptx`
- Python: `oxide.pdf_to_pptx("input.pdf", output="output.pptx")`
- C ABI: `oxide_document_to_pptx(document, 1, &out, &error)`

Default policy:

- One PDF page becomes one slide.
- Text blocks become positioned text boxes.
- Tables become native PPTX DrawingML table shapes.
- Decodable image XObjects become picture shapes. Pass `--no-images` in the CLI
  or `include_images=False` in Python to skip image export.

Overflow policy: content is placed using PDF geometry scaled into the slide. The
exporter does not shrink all text to illegibility to force a pixel-perfect
preview. Dense pages remain editable and may need manual slide cleanup, which is
preferable to rasterizing the page.

Validation on this host:

- Sample slice: `paper.pdf`, `report_multicol.pdf`, `tables.pdf`,
  `invoice.pdf`, and `tracemonkey.pdf`.
- Independent readback: `python-pptx`.
- Slide counts matched parsed page counts for all files.
- Native table-shape counts matched parsed table-block counts for all files.
- Non-table text presence overlap was `1.0` on the sample slice.
- ZIP package integrity: `zipfile.testzip()` returned no bad entries.

## PDF to DOCX

Entry points:

- Rust: `oxide_engine::pdf_to_docx(&engine, &DocxOptions { ... })`
- CLI: `oxide pdf-to-docx input.pdf --out output.docx`
- CLI page-faithful mode: `oxide pdf-to-docx input.pdf --layout page-faithful --out output.docx`
- Python: `oxide.pdf_to_docx("input.pdf", output="output.docx")`
- C ABI: `oxide_document_to_docx(document, 1, &out, &error)`

Default policy:

- The canonical document hierarchy supplies reading order, paragraphs, headings,
  lists, tables, and figures.
- `DocxLayout::Flowing` is the default editable Word mode.
- `DocxLayout::PageFaithful` emits positioned `wp:anchor`/`wps:txbx` blocks for
  geometry-sensitive output while keeping confident tables native.
- Titles/headings become native DOCX paragraph styles.
- Lists become native DOCX numbering definitions.
- Tables become native DOCX tables with `gridSpan` / `vMerge` where the table
  model carries spans.
- Decodable PDF image XObjects become inline DOCX pictures.

Fidelity ceiling: PDF-to-DOCX is a layout reconstruction problem. Oxide targets
strong, editable structure and useful reading order, not pixel-perfect Word
layout or indistinguishability from a document originally authored in Word.
Known weak cases remain sidebars, complex floats, multi-section headers/footers,
and documents whose visual order is ambiguous without human judgment.

Validation on this host:

- Focused round-trip test uses `tracemonkey.pdf`.
- Generated DOCX ZIP contains `word/document.xml`, styles, numbering, and
  native paragraphs/tables.
- The produced DOCX is parsed back by Oxide's native DOCX-to-PDF path and the
  generated PDF reopens through `ContentEngine`.

## Office to PDF

Entry points:

- Rust: `docx_to_pdf`, `xlsx_to_pdf`, `pptx_to_pdf`
- CLI: `oxide docx-to-pdf input.docx --out output.pdf`
- CLI: `oxide xlsx-to-pdf input.xlsx --out output.pdf`
- CLI: `oxide pptx-to-pdf input.pptx --out output.pdf`
- Python: `oxide.docx_to_pdf(...)`, `oxide.xlsx_to_pdf(...)`,
  `oxide.pptx_to_pdf(...)`
- C ABI: `oxide_docx_to_pdf`, `oxide_xlsx_to_pdf`, `oxide_pptx_to_pdf`

The default implementation is native and uses Oxide's PDF authoring/writer
machinery. LibreOffice is documented as a possible future optional backend, but
it is not a default dependency.

Native scope:

- XLSX: rows/columns/cells laid out as PDF tables with pagination windows.
- PPTX: one slide becomes one PDF page; text boxes, simple tables, and PNG/JPEG
  images are placed from slide coordinates.
- DOCX: paragraphs, headings, lists, tables, and inline images flow into PDF
  pages through `FlowDocument`.

Limits:

- XLSX formulas and advanced conditional formatting are not evaluated.
- PPTX animations, SmartArt, charts, theme effects, and complex shape styling
  are not fully rendered.
- DOCX exact Word pagination, complex floats, section-specific furniture,
  equations, and revision markup are outside the native baseline.
