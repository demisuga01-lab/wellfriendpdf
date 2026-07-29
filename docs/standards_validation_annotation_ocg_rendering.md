# Annotation Ocg Rendering Standards Validation

Annotation Ocg Rendering adds a veraPDF-style validation framework with profile id, rule id, severity, object/page location, status, and message. It supports JSON and human output.

## Profiles

- `pdfa`: wraps the existing PDF/A subset checks for output intent, encryption, fonts, active content, metadata, and attachments where implemented.
- `pdfua`: wraps StructTree/MCID/tag/alt-text accessibility subset checks.
- `pdfx`: checks PDF/X output intent, active-content absence, and page TrimBox subset diagnostics.
- `security`: maps Annotation Ocg Rendering risky-content and encryption findings to validation rules.
- `all`: runs every supported subset and includes Arlington status.

## Arlington

Generated Arlington tables are included in every standards report as an informational rule. Unsupported Arlington predicates are reported honestly; full predicate evaluation is not claimed.

## Certification Boundary

`certification_claimed` is always `false` in Annotation Ocg Rendering reports. Wellfriend provides standards-aware diagnostics and supported-subset gates, not legal certification.
