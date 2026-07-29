# Semantic Closeout RAG Chunk Schema

The chunk-set schema is `semantic_closeout.rag_chunk.v1`.

## Chunk-Set Fields

- `schema_version`
- `deterministic`
- `raw_text_rewritten`
- effective `options`
- document title where available
- dictionary and ParentTree status
- document-level security posture
- ordered `chunks`
- diagnostics

## Chunk Fields

Identity and content:

- `chunk_id`, zero-based `index`, `stable_order`, `stable_hash`
- `chunk_type`, `mode`, `page_range`, and `pages`
- raw `text` and whitespace-normalized `normalized_text`
- `token_count_estimate`, `oversized`, and actual overlap token count
- aggregate confidence

Provenance:

- source spans with page, block, semantic block/line/span indexes, bbox, quad,
  character range, MCIDs, role, confidence, and provenance flags;
- page/block citations;
- bounding boxes, quads, block IDs, table/cell IDs, figure/caption IDs;
- heading/section and structure-tree paths;
- MCIDs and ParentTree recovery status/diagnostics.

Language and structure:

- CJK token-layer enablement, token text/ranges/language/confidence/source, and
  dictionary metadata;
- table fragments with row, cell, headers, serializations, and merged-cell
  preservation.

Security:

- original/sanitized state;
- redaction applied;
- removed-content inclusion (always false in current constructors);
- hidden/active-content warnings;
- signature status and diagnostics.

The stable SHA-256 includes text, pages, structural IDs, source span IDs,
structure path, MCIDs, ParentTree status, dictionary hashes, and security state.
It is intended for deterministic indexing, not as a signature or trust claim.

The machine schema is
`target/semantic_closeout-semantic-closeout/rag-chunk-schema-semantic_closeout.json`.
