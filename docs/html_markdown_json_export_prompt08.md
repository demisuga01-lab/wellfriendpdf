# HTML, Markdown, JSON, And RAG Exports In Prompt 08

Prompt 08 adds editable-model based exports alongside the existing parse and
pdftohtml-style surfaces.

New CLI surfaces:

```powershell
wellfriendpdf pdf-to-html input.pdf --out output.html
wellfriendpdf pdf-to-markdown input.pdf --out output.md
wellfriendpdf pdf-to-json input.pdf --out output.json
wellfriendpdf export-editable-model input.pdf --out editable-model.json
```

HTML:

- semantic HTML mode from `EditableDocument::to_semantic_html`.
- headings, paragraphs, lists, and tables.
- safe escaping.

Markdown:

- headings, paragraphs, lists, images, and tables.
- deterministic ordering from the parse/model reading order.

JSON:

- full `EditableDocument` JSON, including schema version, pages, blocks,
  sections, tables, images, diagnostics, and transaction log.

RAG:

- Prompt 06B chunking remains the chunking authority.
- Prompt 08 provides stable editable IDs, section boundaries, and provenance
  summaries that future chunking/citation surfaces can reuse.

Bounded limits:

- page-faithful absolutely positioned HTML is still served by existing
  `to-html` modes, not the new semantic exporter.
- large detailed JSON should be requested intentionally; the default editable
  JSON is structural, not a full glyph dump.
