# Prompt 34 Known Limits

Prompt 34 refuses ambiguous tables, unapproved formula replacement,
low-confidence OCR reconstruction, unsafe appearance generation, unsupported
form actions, and dynamic XFA conversion. These refusals preserve input bytes.

The table writer edits provenance-resolved nonempty cells; it does not invent a
row/column/merged-cell topology from decorative geometry. Formula edits cover
resolved born-digital text and retain outlined/raster formula artwork pending
review. New OCR searchable text is limited to reviewed ASCII provider results
on image-only pages until a canonical `/ToUnicode` OCR font route is selected.
Signature values, dynamic XFA, unsupported widget/annotation types, and unsafe
actions are exact no-change boundaries.

Static XFA data import requires an indirect, parseable `/datasets` member of a
packet array and proves all other decoded packets unchanged. Direct or
single-stream XFA, malformed arrays, and dynamic templates are refused.
