# Accessibility Repair After Mutations

Prompt 35 provides a repair-after-mutation action for changes introduced by
earlier true-editing prompts. The repair path records the mutation class,
applies the canonical PDF/UA best-effort repair when supported, validates the
result, and includes accessibility effects in the operation report.

Mutation classes include content reflow, table editing, math editing, OCR
searchable-layer changes, annotation edits, form edits, redaction, and
sanitization. The engine refuses unsupported structure updates with exact typed
limits rather than silently discarding tags or reading-order links.
