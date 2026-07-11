# Prompt 19 Interactive / Data Scorecard

Prompt 19 removes the vague “interactive edge cases later” bucket. The public
`interactive_data_report` composes the existing AcroForm/widget, FDF/XFDF, XFA,
annotation, page-operation, redaction, associated-file, security, optional-
content, deterministic-writer, and Prompt 18B signature-policy surfaces.

Implemented consistency fields include deterministic stable IDs, object/page
provenance, field/widget ownership, annotation page ownership, associated-file
owners, sanitizer disposition, signature impact, and deterministic JSON.

Exact remaining limits:

- dynamic XFA JavaScript stays inventory-only; bounded XFA/FormCalc handling is
  separate and opt-in;
- OCG-dependent visibility is reported/preserved but not promoted into an
  Acrobat UI state machine;
- popup/reply and appearance edge cases retain Prompt 17 exact limits;
- cryptographic signature validity comes only from the signature verifier;
- cross-feature mutation scenarios that require a prohibited DocMDP/FieldMDP
  change fail closed.

The generated scorecard has `blocked: 0` and `unclassified_failures: 0`.
