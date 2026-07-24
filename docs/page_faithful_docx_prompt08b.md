# Page-Faithful DOCX In Prompt 08B

Prompt 08B adds `DocxLayout`:

- `Flowing`: default Prompt 08 semantic DOCX.
- `PageFaithful`: positioned text boxes/images for geometry-sensitive output.
- `Hybrid`: native semantic blocks where confident, positioned fallback otherwise.

CLI:

```powershell
wellfriendpdf pdf-to-docx input.pdf --layout page-faithful --out output.docx --json
```

Implementation:

- `DocxOptions { include_images, layout }`.
- page-faithful text blocks are emitted as anchored OOXML `wp:anchor` drawings with `wps:txbx` text boxes.
- page-faithful images are emitted as anchored picture drawings.
- confident tables remain native DOCX tables so spans/header rows are still available.
- flowing mode remains the default and existing readback tests stay unchanged.

Verification:

- tests inspect `word/document.xml` for `wp:anchor` and `wps:txbx`.
- package readback still opens the DOCX ZIP parts.

Limits:

- exact Word pagination is not guaranteed.
- some OOXML readers expose text box text differently from normal paragraphs.
- image crop/mask reconstruction remains bounded.
