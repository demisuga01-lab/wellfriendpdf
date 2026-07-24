# Prompt 07B FDF/XFDF Form Exchange

Wellfriend now supports bounded AcroForm field exchange through JSON, FDF, and XFDF.
The supported scope is intentionally fields-only.

## Supported

- Export text, checkbox/radio, and choice field names and values.
- Import matching field names and update AcroForm values.
- Regenerate common widget appearances through the existing form fill path.
- Deterministic FDF writer using minimal `%FDF-1.2` syntax.
- XFDF writer with XML escaping.
- XFDF parser rejects DTDs, external entities, `SYSTEM`, and `PUBLIC`.
- Input cap: 4 MiB form data.
- Field cap: 10000 fields.

## API and CLI

- Rust: `export_form_data`, `parse_form_data`, `apply_form_data_pdf`.
- CLI:

```powershell
wellfriendpdf forms-export input.pdf --format xfdf --output fields.xfdf
wellfriendpdf forms-import input.pdf fields.xfdf --format xfdf --out filled.pdf --json
```

## Limits

- Annotation FDF/XFDF is not imported or exported in Prompt 07B.
- Acrobat JavaScript, submit actions, and calculation scripts are never
  executed.
- Signature fields are skipped for import.

