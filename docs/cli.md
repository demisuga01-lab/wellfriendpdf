# Wellfriend CLI

Wellfriend's CLI is a scriptable interface over the Rust engine. Human-readable output remains the default where it already existed; machine-readable output is opt-in through `--json` or `--format json`.

## Exit Codes

| code | name | meaning |
| ---: | --- | --- |
| 0 | success | Command completed successfully. |
| 1 | internal error | Unexpected internal failure or caught panic. This is a bug report candidate. |
| 2 | usage error | Invalid flags, unknown format/profile/type, invalid page range, or incompatible options. Clap argument errors also use code 2. |
| 3 | I/O error | Input/output path could not be read or written. |
| 4 | parse/format error | The file is malformed, encrypted without the right password, resource-limited, or otherwise rejected as input. |
| 5 | unsupported feature | The request needs a feature this build or command does not support, such as OCR in a non-OCR build. |

Malformed PDFs should return a clean non-zero exit code and an `wellfriendpdf: <category>: <message>` stderr line. Raw Rust panic text should not reach users.

## Machine Output

Use JSON for scripts:

```powershell
wellfriendpdf info input.pdf --json
wellfriendpdf fonts input.pdf --json
wellfriendpdf detach input.pdf --list --json
wellfriendpdf verify-sig signed.pdf --json
wellfriendpdf extract-text input.pdf --structured --format json
wellfriendpdf extract-tables input.pdf --format json --structure
wellfriendpdf parse input.pdf --format json
wellfriendpdf extract-fields input.pdf --format json
wellfriendpdf chunk input.pdf --format json
```

File-writing commands that naturally write their primary artifact to disk expose JSON result summaries:

```powershell
wellfriendpdf render input.pdf --format png --output pages.zip --json
wellfriendpdf pdf-to-jpg input.pdf --out-dir pages --dpi 150 --quality 85 --json
wellfriendpdf pdf-to-jpg input.pdf --out-dir pages-png --format png --json
wellfriendpdf image-to-pdf scan1.jpg scan2.png --out wrapped.pdf --page-size a4 --json
wellfriendpdf pdf-to-xlsx tables.pdf --out tables.xlsx --layout pages --json
wellfriendpdf pdf-to-pptx report.pdf --out slides.pptx --json
wellfriendpdf pdf-to-docx report.pdf --out report.docx --json
wellfriendpdf docx-to-pdf report.docx --out report.pdf --json
wellfriendpdf xlsx-to-pdf tables.xlsx --out tables.pdf --json
wellfriendpdf pptx-to-pdf slides.pptx --out slides.pdf --json
wellfriendpdf extract-images input.pdf --output images.zip --json
wellfriendpdf merge a.pdf b.pdf --output merged.pdf --json
wellfriendpdf split input.pdf --output page-%d.pdf --json
wellfriendpdf extract-pages input.pdf 1,3-5 --output subset.pdf --json
wellfriendpdf organize input.pdf --order 1,2,5,3,4,3 --output organized.pdf --json
wellfriendpdf linearize input.pdf --output linearized.pdf --json
wellfriendpdf encrypt input.pdf --user-pw secret --output encrypted.pdf --json
wellfriendpdf decrypt encrypted.pdf --password secret --output unlocked.pdf --json
wellfriendpdf rotate input.pdf --angle 90 --output rotated.pdf --json
wellfriendpdf watermark input.pdf --text CONFIDENTIAL --opacity 0.3 --rotation 45 --output watermarked.pdf --json
wellfriendpdf add-page-numbers input.pdf --format "Page {n} of {total}" --output numbered.pdf --json
wellfriendpdf optimize input.pdf --output optimized.pdf --json
wellfriendpdf repair damaged.pdf --output repaired.pdf --json
```

These summaries use stable top-level fields:

| field | type | meaning |
| --- | --- | --- |
| `op` | string | Command operation name. |
| `output` | string | Output path when the command writes one primary artifact. |
| `bytes` | number | Output byte length when known. |
| `pages`, `pages_requested`, `pages_rendered` | number/array | Page counts or selected pages, depending on command. |
| `images`, `inputs`, `files` | number | Command-specific counts. |

## Phase 3 Document Utilities

`pdf-to-jpg` rasterizes whole pages through the same renderer used by `render`;
it is not the same as `extract-images`, which extracts embedded image XObjects.
Use `--format png` when lossless page screenshots are needed. Pages are rendered
and written one at a time.

`image-to-pdf` accepts JPG and PNG inputs and writes one image per page. Page
size can be `a4`, `letter`, or `size-to-image`; images are fit to the page while
preserving aspect ratio.

`watermark` and `add-page-numbers` append overlay content streams to existing
pages. They do not rasterize and replace the page, so existing digital text
remains searchable/extractable. A text watermark may appear in later text
extraction because it is real page text.

`organize` copies pages in the exact 1-based order supplied by `--order`.
Repeated indices duplicate pages; omitted indices delete pages. `--insert-from`
can insert pages from a second document at `--insert-at`.

Command-specific extraction JSON schemas are documented by the command outputs themselves and covered by integration tests. They are treated as compatibility surfaces for scripts.

## Phase 4 Office Conversions

`pdf-to-xlsx` reads the canonical document hierarchy and writes detected table
blocks into a valid XLSX workbook. The default `--layout pages` creates one
worksheet per PDF page and keeps non-table text as context rows near the tables.
`--layout tables` creates one worksheet per detected table and puts non-table
text on a `Notes` sheet. Numeric-looking cells are written as numbers when this
can be inferred safely; leading-zero identifiers remain text.

`pdf-to-pptx` maps one PDF page to one slide. Text blocks become positioned text
boxes, tables become native PPTX table shapes, and decodable image XObjects
become picture shapes unless `--no-images` is passed. It preserves editable
structure, not pixel-perfect PDF appearance.

`pdf-to-docx` reconstructs a document from the same hierarchy. The default
`--layout flowing` writes native headings, paragraphs, lists, tables, and inline
images. `--layout page-faithful` uses positioned OOXML text boxes and anchored
images to preserve page geometry where a flowing document would lose too much
layout. It still does not claim pixel-perfect Word pagination.

`docx-to-pdf`, `xlsx-to-pdf`, and `pptx-to-pdf` are native by default and reuse
Wellfriend's authoring/writer machinery. They do not require LibreOffice. Their
fidelity boundaries and the optional external-renderer decision are documented
in `docs/office_to_pdf_architecture.md`.

These conversions are built on `docs/document_hierarchy.md`; they do not run a
second table detector and do not change extraction accuracy paths.

## OCR Honesty

Default builds report OCR as unavailable in `wellfriendpdf --version`. Commands that need OCR return exit code 5 with an actionable message unless the CLI is rebuilt with `--features ocr` and the external `tesseract` binary plus language data are installed. `extract-tables --ocr` is intentionally unsupported today because reconstructing table grids from OCR word boxes is a known gap; use `extract-fields --ocr` or `extract-text --ocr` for scanned documents.

## Help

Top-level help groups commands by purpose:

```powershell
wellfriendpdf --help
wellfriendpdf extract-text --help
wellfriendpdf render --help
```

Region coordinates are PDF user-space points with the origin at the bottom-left, matching the region extraction docs.
