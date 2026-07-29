# PDF To DOCX In Advanced Rendering

DOCX export is native Rust and does not shell out to LibreOffice. Advanced Rendering
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

Type3 CID Rendering closure:

- `DocxLayout::Flowing` remains the default semantic DOCX mode.
- `DocxLayout::PageFaithful` emits positioned `wp:anchor` / `wps:txbx` text
  boxes for text blocks and anchored image drawings.
- `DocxLayout::Hybrid` keeps confident semantic structures and positions lower
  confidence blocks.
- CLI: `wellfriendpdf pdf-to-docx input.pdf --layout page-faithful --out output.docx`.
- tests inspect the generated OOXML for anchored text box markup.

Bounded limits:

- full Word style recreation is approximate.
- exact PDF line breaks are not always desirable in flowing DOCX mode.
- headers/footers and footnotes are exported only when the reconstructed model
  exposes them confidently.
- OOXML readers differ in how they expose text box text through high-level APIs.

CLI:

```powershell
wellfriendpdf pdf-to-docx input.pdf --out output.docx --json
wellfriendpdf pdf-to-docx input.pdf --layout page-faithful --out output.docx --json
```

Advanced Rendering also keeps the existing `docx-to-pdf` native authoring path unchanged.
