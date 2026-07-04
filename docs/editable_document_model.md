# Editable Document Model

The editable model is the shared Prompt 08 bridge between fixed PDF drawing
instructions and outputs that need editable structure. It consumes the existing
canonical `parse::Document`, semantic/layout results from Prompt 06B, table
results from Prompt 07, and image discovery from the renderer/image locator.

Pipeline:

```text
PDF bytes -> ContentEngine -> parse::Document -> editable::EditableDocument
          -> PDF edits / DOCX / PPTX / XLSX / HTML / Markdown / JSON
```

Main structures:

- `EditableDocument`: schema version, source parse document, sections, pages,
  blocks, diagnostics, and transaction log.
- `EditablePage`: source page size and block/image IDs.
- `EditableBlock`: role, bbox, reading order, confidence, edit-safety level,
  paragraphs, optional table/image, and provenance.
- `EditableParagraph`: logical text, runs, list metadata, confidence.
- `EditableRun`: segmented text with bold/italic/link style placeholders.
- `EditableTable`: rows, columns, cells, row/column spans, header flag.
- `EditableImage`: source placeholder, bbox, intrinsic metadata when known.

Edit-safety levels:

- `safe_patch`: additive overlays or metadata-only changes.
- `local_reflow_rewrite`: paragraph/run replacement by re-typesetting.
- `page_regenerate`: page-level regeneration may be needed.
- `unsupported`: no safe edit path is currently claimed.

The current transaction log is in-memory and deterministic. It provides
undo/redo for block text replacement in the model. It is not yet a persistent
HAMT/RRB snapshot store; that remains a later optimization.

JSON stability:

- `schema_version` is `0.1`.
- IDs are deterministic (`page-N`, `block-N`, `section-N`).
- Detailed provenance is conservative and source-derived; it does not invent
  unsupported structure.

Known bounded limits:

- The model does not claim perfect paragraph reconstruction for arbitrary PDFs.
- Exact per-character style/color is retained in the Prompt 06B semantic model
  but only summarized at editable run level in Prompt 08.
- Image export currently records placeholders and page image IDs; exact image
  crop/mask export is a later conversion polish item.
