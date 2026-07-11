# Signature-aware incremental mutation

Form value, annotation add/update, and page rotation/CropBox mutations call the structural signature-policy analyzer before writing. Default policy enforces blocks; an explicit override is required to proceed past one.

Allowed mutations append a revision with the exact input as prefix. Output is reopened and the field, annotation, or page property is verified. A post-save impact report is generated. Prefix preservation means original covered bytes and signature dictionaries were not rewritten; it does not establish validity, trust, certification acceptance, or absence of viewer warnings.

Secure redaction, sanitizer removal, destructive attachment cleanup, XFA flatten/remove, and canonicalization remain honestly invalidating full rewrites.
