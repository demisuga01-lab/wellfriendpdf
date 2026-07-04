# PDF To XLSX In Prompt 08

XLSX export consumes the Prompt 07 table model through the shared editable-model
route. The writer remains native Rust and emits OpenXML workbooks.

Supported:

- `pages` layout: one worksheet per PDF page.
- `tables` layout: one worksheet per detected table plus notes for non-tabular
  content.
- cell text, rows, columns, and span metadata where the table model exposes it.
- readback verification by opening `xl/workbook.xml` from the XLSX ZIP package.

Bounded limits:

- numeric/date type inference remains conservative.
- complex cell styling is not a Prompt 08 goal.
- merged-cell styling is less important than preserving grid structure and
  source text.

CLI:

```powershell
oxide pdf-to-xlsx input.pdf --layout pages --out output.xlsx --json
oxide pdf-to-xlsx input.pdf --layout tables --out tables.xlsx
```
