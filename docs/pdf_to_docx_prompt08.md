# PDF To DOCX In Prompt 08

DOCX export is native Rust and does not shell out to LibreOffice. Prompt 08
routes PDF-to-DOCX through the editable-model reconstruction path:

```text
ContentEngine -> parse::Document -> EditableDocument -> parse::Document -> DOCX writer
```

Supported:

- flowing DOCX with paragraphs, headings, runs, tables, and inline image
  placeholders where available.
- bold/italic/link span data from the parse model.
- table grids from the shared table model.
- package readback verification by opening `word/document.xml` from the DOCX
  ZIP package.

Bounded limits:

- page-faithful Word text boxes/frames are not implemented in Prompt 08.
- full Word style recreation is approximate.
- exact PDF line breaks are not always desirable in flowing DOCX mode.
- headers/footers and footnotes are exported only when the reconstructed model
  exposes them confidently.

CLI:

```powershell
oxide pdf-to-docx input.pdf --out output.docx --json
```

Prompt 08 also keeps the existing `docx-to-pdf` native authoring path unchanged.
