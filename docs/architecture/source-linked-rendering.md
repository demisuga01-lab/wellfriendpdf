# Source-linked rendering

Rendering is tied to the same provenance model used by true editing:

- bytes and revisions;
- COS objects;
- content-stream instructions;
- display items;
- editable scene nodes;
- semantic graph nodes;
- transaction and validation reports.

After an edit, Wellfriend identifies changed source objects, invalidates the
dependent display-list nodes and tiles, recompiles only affected units where
possible, renders affected pages/tiles, serializes through the canonical writer,
reopens output, and records transaction provenance.

Renderer-only optimization must not break extraction, redaction, reflow, undo,
tagged-PDF repair, signatures, standards reports, or source-level editability.
