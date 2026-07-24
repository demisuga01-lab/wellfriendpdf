# PDF To PPTX In Prompt 08

PPTX export maps PDF pages to slides. Prompt 08 keeps the native PPTX writer and
routes the conversion through the editable model so it shares reconstruction
with DOCX/XLSX/HTML/Markdown/JSON.

Supported:

- one PDF page per slide.
- positioned text shapes based on recovered block geometry.
- images when decodable and `include_images` is enabled.
- table shapes where the existing writer can express them.
- readback verification by opening `ppt/presentation.xml` from the PPTX package.

Bounded limits:

- arbitrary vector paths are not fully converted to editable PowerPoint shapes.
- transparency, clipping, and complex masks are preserved only where the current
  writer supports them.
- animation and slide master reconstruction are out of scope.

CLI:

```powershell
wellfriendpdf pdf-to-pptx input.pdf --out output.pptx --json
```
