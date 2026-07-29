# Shared Document Hierarchy for Office Conversion

This document is the Phase 4a decision record for PDF-to-Office conversion.
It is authoritative for the Phase 4b-4e converters.

## Decision

Wellfriend will use the canonical parsed document model (`parse::Document`, built
from `docmodel.rs`) as the shared hierarchy for PDF-to-Word and
PDF-to-PowerPoint. PDF-to-Excel will consume the same document model, but its
primary input is the table/grid projection carried by `BlockKind::Table` and
`analysis::tables::Table`.

This is option (c) from the Phase 4a roadmap task:

- `docmodel.rs` remains the backbone for flowing and page-positioned document
  content: headings, paragraphs, lists, figures, captions, tables, page
  geometry, reading order, and provenance.
- Excel is grid-shaped, not flow-shaped. The correct shared input for Excel is
  the table model already attached to document table blocks: rows, origin
  cells, row/column coordinates, `rowspan`/`colspan`, header flags, header
  hierarchy, confidence, and table bounding boxes.
- No extraction behavior was changed for this decision. The Phase 4b/4c
  conversion code is additive and reads the existing hierarchy.

## Actual Model Shape

`docmodel.rs` builds a tags-first, geometric-fallback model:

- `DocumentModel`: source, page count, body font size, ordered `DocBlock`s.
- `DocBlock`: stable id, classified type, page, PDF-space bounding box,
  reading-order index, text, confidence, source basis, list items, caption/figure
  links, furniture/page-number flags, bold/italic flags, optional table.
- `ClassifiedType`: title, heading, paragraph, list, list item, figure, caption,
  table, header, footer, page number, and fallback text.
- `analysis::tables::Table`: flattened rows plus span-aware cells, header
  metadata, nested tables, detection source, confidence, bbox, and notes.

`parse.rs` wraps this into the public canonical hierarchy:

- `Document`: schema version, metadata, source provenance, flattened ordered
  `body`, and `pages`.
- `Page`: page number, page width/height, source classification, and block ids.
- `Block`: id, page, bbox, reading order, confidence, and a typed `BlockKind`.
- `BlockKind`: title, heading, paragraph, list, figure, caption, table, header,
  footer, page number, and fallback text.
- `InlineText`/`InlineSpan`: plain text plus bold/italic/link span metadata.

The hierarchy is not a nested DOM tree. It is a flat reading-ordered block
stream with page views, explicit geometry, and cross-links. That is a better fit
for PDF and PowerPoint than a pure flow tree, and it still serializes cleanly to
Markdown/HTML.

## Corpus Evidence

The decision was checked against real repository fixtures using
`wellfriendpdf parse --format json` and summarized under `target/phase4a-prototype`.

| PDF | Pages | Relevant output |
| --- | ---: | --- |
| `paper.pdf` | 1 | 3 headings, 3 paragraphs |
| `report_multicol.pdf` | 1 | heading plus paragraph in recovered reading order |
| `tables.pdf` | 1 | 1 paragraph, 1 table, 3x3 rows, 9 cells, 5 headers |
| `invoice.pdf` | 1 | 2 headings, 2 paragraphs, 1 table, 3x4 rows, 12 cells, 6 headers |
| `receipt.pdf` | 1 | heading, paragraphs, 1 table, 3x2 rows, 6 cells, 4 headers |
| `figure.pdf` | 1 | heading and paragraphs; no decodable figure block in this fixture |
| `form_160f.pdf` | 2 | tagged source; paragraph blocks plus 1 large table, 52x5 rows, 117 cells |
| `tracemonkey.pdf` | 14 | headings, paragraphs, fallback text, 6 lists, 1 table, page geometry |

Findings:

- Word/PPTX need headings, paragraphs, lists, tables, figures, reading order,
  page geometry, and basic inline styling. The canonical model already carries
  these.
- PPTX needs absolute positions. `Block::bbox` and `Page` dimensions provide
  that. The converter defensively falls back to engine page dimensions when a
  parsed page reports missing geometry.
- XLSX needs a grid. `analysis::tables::Table` already carries the grid data
  Excel needs, including origin cells, spans, and headers. Non-table text is
  not naturally grid data and is therefore placed as notes/context, not forced
  into fabricated table cells.
- Forms can be represented as tables when the existing extractor recognizes
  their layout. That is useful for Excel, but not equivalent to full AcroForm
  semantics.

## Gaps and Fidelity Notes

These gaps are explicit inputs to the Office conversion prompts:

- DOCX lists: the model has list entries and markers, but not full numbering
  definitions. DOCX export should generate a simple numbering definition from
  the list metadata.
- DOCX sections/headers/footers: the model classifies furniture but does not
  preserve complete section-break semantics. Export should preserve furniture
  as normal content unless a later section model is added.
- PPTX placeholders: the model identifies headings but not authored slide title
  placeholders. PPTX export may use a heuristic for dominant top headings.
- PPTX images: figure blocks identify figure regions; decodable image XObjects
  are available through the image locator. Export can use both, but should not
  pretend vector drawings are raster images.
- XLSX formulas: extracted PDF text cannot prove a cell was originally a
  formula. Numeric-looking cells may become numbers; formulas remain text.
- Rich inline styling: `InlineSpan` has bold/italic/link, but not the full PDF
  font/color model. Office exports preserve basic formatting where available.

## Worked Mapping

### DOCX

| Hierarchy node | DOCX mapping | Notes |
| --- | --- | --- |
| `Title` / `Heading { level }` | `<w:p>` with heading style (`Title`, `HeadingN`) | Clean mapping for text; PDF position is usually ignored in flowing mode. |
| `Paragraph { InlineText }` | `<w:p>` with one or more `<w:r>` runs | Bold/italic/link spans map to run properties. |
| `List` | `<w:p>` per item with numbering properties | List depth is weak today; nested lists need future metadata. |
| `Table` | `<w:tbl>` with rows/cells and `gridSpan`/`vMerge` | Clean for detected tables; visual cell widths are approximate. |
| `Figure` / image XObject | `<w:drawing>` image | Clean when bytes are decodable; vector-only figures are lossy. |
| Header/footer/page number blocks | Normal paragraphs or section header/footer if a future section model exists | Current model lacks full section ownership. |

### PPTX

| Hierarchy node | PPTX mapping | Notes |
| --- | --- | --- |
| PDF page | One slide | Clean structural match: both are positioned canvases. |
| `Title` / dominant top `Heading` | Text box, optionally title-like styling | Placeholder identity is heuristic. |
| `Paragraph` / fallback `Text` | Text box at scaled `Block::bbox` | Clean for position; text may wrap differently. |
| `List` | Text box with one line per list item | Basic bullets/numbers are preserved as text. |
| `Table` | Native DrawingML table in a positioned `graphicFrame` | Structure is preserved; spans are currently flattened in PPTX output. |
| Image XObject | Picture shape with relationship to `/ppt/media/imageN.png` | Decode failures are contained to that image. |

### XLSX

| Hierarchy node | XLSX mapping | Notes |
| --- | --- | --- |
| `Table` | Worksheet cells by row/column | Primary conversion path; headers are bold. |
| `TableCell.rowspan` / `colspan` | Excel merged ranges | Clean when the table model carries spans. |
| Header cells | Bold cell style | Header hierarchy is not yet converted into Excel outline groups. |
| Numeric-looking text | Number cell where safely inferable | Leading-zero identifiers remain text. |
| Non-table text | Context rows or a `Notes` worksheet | Honest grid compromise; prose is not fabricated into a table. |
| Images/figures | Not exported by Phase 4b | Excel export is table/data first. |

## Streaming and Memory

The hierarchy remains constructable through the existing page-oriented parser and
bounded rendering/extraction infrastructure. The Office writers emit OOXML ZIP
parts one sheet/slide at a time and do not render pages to bitmaps. They do not
change extraction scoring paths.

Large-document conversions should continue to be validated under the Phase 1
2 GB cap. The current model is still a materialized block stream, so a future
lazy page iterator remains a useful improvement for very large born-digital
documents; no Phase 4 conversion should introduce image-page accumulation.
