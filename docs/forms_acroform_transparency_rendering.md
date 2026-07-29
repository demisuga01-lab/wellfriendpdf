# Transparency Rendering Forms and AcroForm

Transparency Rendering adds an auditable form report on top of the existing exact AcroForm
extraction and editing paths.

## Supported

- Catalog `/AcroForm` detection.
- Field tree traversal with a depth cap.
- Full field name construction.
- Inherited field attributes: `/FT`, `/Ff`, `/DA`, `/DR`, `/Q`, `/Opt`,
  `/MaxLen`, `/V`, and `/DV`.
- Widget mapping to pages through page `/Annots`.
- Widget rects, annotation flags, and appearance-stream presence.
- Field types: text, checkbox, radio, push button, choice, signature, and
  unknown.
- `NeedAppearances`, `/SigFlags`, and calculation-order length reporting.
- Basic fill and flatten through `PdfEditor`.
- Fields-only JSON/FDF/XFDF export and import through Transparency Closeout.
- XFA detection and packet count reporting.
- JavaScript action detection without execution.

## CLI/API

- Rust: `forms_report(&ContentEngine)`.
- CLI: `wellfriendpdf forms-report input.pdf`.
- CLI: `wellfriendpdf forms-export input.pdf --format xfdf --output fields.xfdf`.
- CLI: `wellfriendpdf forms-import input.pdf fields.xfdf --format xfdf --out filled.pdf`.
- Existing editing API: `PdfEditor::set_form_text`,
  `set_form_checkbox`, `set_form_choice`, and `flatten_forms`.

## Limits

- Dynamic XFA is detected and reported as unsupported. Wellfriend does not execute
  XFA or Acrobat JavaScript.
- Signature fields are detected, but cryptographic validation is Annotation Ocg Rendering.
- Rich text field appearance and all producer-specific DA quirks are bounded
  appearance-generation work.
- FDF/XFDF support is fields-only; annotation XFDF and JavaScript-driven
  calculation/import are intentionally unsupported.
