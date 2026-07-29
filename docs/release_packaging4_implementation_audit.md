# document subsystems Implementation Audit

document subsystems extends the canonical engine rather than introducing parallel PDF
systems. `analysis::tables` and `table_intelligence` supply table evidence;
`ocr` and the OCR binding adapters supply provider and preprocessing seams;
`interactive`, `editing`, `extract::acroform`, `form_exchange`, and
`annotation_media_redaction` supply annotation, appearance, AcroForm, and XFA seams. source editing
provenance, editing transactions transactions, text reflow reflow, and `writer` remain the
only mutation, undo, layout, and serialization paths.

The implementation is consolidated in a document subsystems runtime module and SDK
envelopes. It records source links, typed limits, transaction effects, and
reopen-safe output for supported compact fixtures. Bindings call those SDK
envelopes; they do not recreate table, math, OCR, annotation, form, or XFA
logic.

Documented boundaries are explicit: dynamic XFA, unapproved raster-formula
replacement, unsupported OCR providers, and annotation/form cases without a
safe canonical appearance path return typed no-change results.
