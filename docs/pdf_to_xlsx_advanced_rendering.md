# PDF To XLSX In Advanced Rendering

XLSX export consumes the Transparency Rendering table model through the shared editable-model
route. The writer remains native Rust and emits OpenXML workbooks.

Supported:

- `pages` layout: one worksheet per PDF page.
- `tables` layout: one worksheet per detected table plus notes for non-tabular
  content.
- cell text, rows, columns, and span metadata where the table model exposes it.
- readback verification by opening `xl/workbook.xml` from the XLSX ZIP package.

Bounded limits:

- numeric/date type inference remains conservative.
- complex cell styling is not a Advanced Rendering goal.
- merged-cell styling is less important than preserving grid structure and
  source text.

CLI:

```powershell
wellfriendpdf pdf-to-xlsx input.pdf --layout pages --out output.xlsx --json
wellfriendpdf pdf-to-xlsx input.pdf --layout tables --out tables.xlsx
```
