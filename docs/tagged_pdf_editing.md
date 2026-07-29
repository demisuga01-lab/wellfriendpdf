# Tagged PDF Editing

Prompt 35 does not create a new tagged-PDF subsystem. It routes tagged structure
inspection through the semantic extractor and applies supported repairs through
the existing PDF/UA improvement path.

Supported structure operations:

- inspect StructTreeRoot, marked content, and recovered semantic evidence;
- set document language;
- set structure metadata where a canonical catalog/update path exists;
- rebuild ParentTree evidence using the semantic recovery path;
- repair structure after table, formula, OCR, annotation, form, redaction, or
  sanitization mutations.

Unsupported structure changes return typed refusal results instead of mutating
unknown structure relationships.
