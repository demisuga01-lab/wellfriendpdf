# Advanced RAG Chunking

Semantic Closeout adds `advanced_rag` alongside the legacy chunk set. It uses the
canonical document for structural boundaries and joins detailed text,
structure, ParentTree, dictionary, table, and security evidence into each
chunk.

## Modes

- `hybrid`: size-bounded structural packing with optional overlap;
- `page`: one group per page;
- `section`: heading-path groups;
- `paragraph`: one paragraph unit per chunk;
- `table`: one structure-preserving table unit;
- `table_row`: row units with header association;
- `table_cell`: origin-cell units with row/column span metadata;
- `figure_caption`: figure and attached caption units;
- `cjk_token_aware`: hybrid splitting at dictionary token boundaries where
  possible;
- `search_index`: deterministic no-overlap indexing chunks.

Atomic structural units are not destructively split. If an atomic table or
figure exceeds the target, it remains whole and `oversized=true` is reported.
Non-atomic oversized text splits at CJK dictionary token boundaries or falls
back to deterministic word boundaries.

Overlap reports actual repeated units, not merely the configured overlap
budget. `overlap_tokens=0` and `search_index` produce no overlap.

## Tables

Table chunks retain table and cell IDs, associated headers, row/cell location,
merged-cell status, source bboxes, and optional Markdown and JSON
serializations. The original deterministic table object is not flattened or
mutated.

## CJK

The CJK Dictionary Layout dictionary provider guides split boundaries. Known words are
kept whole when the requested size permits. Raw text and offsets remain
unchanged, and dictionary pack source/license/version/hash metadata travels
with chunks. Missing or disabled dictionaries fall back deterministically.

## Structure And Citations

Chunks include heading/section and structure-tree paths, source spans, page and
block IDs, bboxes, quads, MCIDs, ParentTree recovery status and diagnostics,
and page/block citations. Bibliography-level reference linking is only present
when the extracted semantic structure supplies it; page/block source citations
are always the primary citation contract.

## Security

Original-input chunks explicitly say they are not asserted sanitized. Hidden
text and active-content warnings are carried when detected. Post-redaction
chunking must use the rewritten bytes; `ChunkSecurityPosture` can then mark the
document sanitized and redacted. Removed content is never reconstructed by the
chunker.
