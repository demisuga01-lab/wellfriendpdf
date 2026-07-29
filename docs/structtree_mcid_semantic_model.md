# StructTree and MCID Semantic Model

Tagged PDFs can describe logical structure through `/StructTreeRoot`, role nodes,
and marked-content IDs. Reference Renderer connects that authored structure to the same
geometry-backed text model used for search and redaction planning.

## Flow

PDF content stream `BDC/EMC` marked text -> `MarkedTextChunk` with page-local
MCID -> `StructTreeRoot` flattening -> `TextStructureContext` -> semantic
chars/spans/lines/blocks.

## Behavior

- MCIDs are page-local keys: `(page, mcid)`.
- RoleMap entries normalize custom roles and preserve `original_role`.
- Chars carry `mcid`, `struct_role`, `original_role`, and `role_source`.
- Spans, lines, blocks, and search matches aggregate MCIDs and provenance.
- Tagged roles override geometric role guesses when directly attached to MCID
  content.
- Duplicate MCID mappings emit `text.structure.duplicate_mcid`.
- Empty mapped content emits `text.structure.empty_mcid`.
- Cap hits emit `text.structure.cap` or `text.structure.mcid_cap`.

## Limits

- ParentTree-only recovery is not required for current fixtures because direct
  `/K` MCID references are already parsed. Broken ParentTree diagnostics remain
  a broader tagged-PDF validation item.
- Artifacts are flagged through structure metadata, but default flat extraction
  remains unchanged.
- Tables/forms/annotations and redaction apply are Transparency Rendering work.

