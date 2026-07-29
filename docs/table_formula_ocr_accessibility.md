# Table, Formula, and OCR Accessibility

Prompt 35 links accessibility repair to Prompt 34 table, mathematical-content,
and OCR operations.

- Table edits can trigger structure repair for table roles, cells, captions, and
  reading order where canonical semantic evidence exists.
- Math edits preserve ActualText/MathML-style evidence when present and report
  unresolved formula semantics for review.
- OCR searchable-layer changes keep the original scan by default and can rebuild
  accessible text evidence for reviewed regions.

The engine does not silently replace low-confidence OCR or inferred formula
semantics with destructive tagged content.
