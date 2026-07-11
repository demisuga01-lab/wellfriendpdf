# DOCX Layout Modes

- `flowing`: native editable paragraphs/lists/tables with explicit source-page
  sections. Best semantic editability; line wrapping follows the editor.
- `page-faithful`: all non-table blocks use page-relative text boxes and images
  use page-relative anchors. Best supported geometry preservation.
- `hybrid`: confident titles, headings, lists, and tables remain native flow;
  other blocks and images use deterministic page-relative anchors.

Rust uses `DocxOptions { include_images, layout }`. CLI uses
`pdf-to-docx --layout flowing|page-faithful|hybrid`. Python accepts `layout=`;
C ABI, .NET, and Java expose explicit-layout overloads. Unsupported layout names
return the stable input/parse error category.
